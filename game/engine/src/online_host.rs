//! Hosting a world for a friend in another house.
//!
//! The host publishes nothing about its world anywhere. It subscribes to
//! kind-20900 events addressed to its own runtime key, and every offer that
//! arrives is either from somebody already in its contacts book or from
//! somebody holding an invite the host minted itself. There is no directory, no
//! listing, and no way to arrive here without having been given something first
//! (CLAUDE.md red line 1).
//!
//! `poll` is called from the game loop and never blocks: relay reads are
//! `try_recv`, and the one blocking thing — punching, which sleeps ~200 ms —
//! goes to a worker thread.
//!
//! The contacts book handed to [`OnlineHost::start`] is a **snapshot**: this
//! module never writes to it or to the mirror (an invite admission is a session
//! allowlist entry and no row) and never re-reads the mirror, so a contact added
//! or re-tiered elsewhere while a world is hosted is not seen here. Tasks 17/18,
//! which own the Friends UI and the mirror, must push a refreshed book in when
//! they change one.
//!
//! Spec: `docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md` §5.1.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::{HashSet, VecDeque};
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nostr::EventId;

use crate::contacts::Contact;
use crate::invite::{DEFAULT_INVITE_TTL_SECS, Invite, mint_bearer};
use crate::nat::candidates::parse_addrs;
use crate::online_admission::{
    ActiveBearer, Admission, AdmitReason, BearerClaim, Refusal, admit, bearer_eq, is_reply_worthy,
    refusal_wire,
};
use crate::rendezvous::payload::{
    Answer, Candidate, KIND_JOIN_ANSWER, KIND_JOIN_OFFER, PAYLOAD_VERSION, npub_of, seal_answer,
};
use crate::access_policy::Allow;
use crate::rendezvous::relay_client::RendezvousRelay;
use crate::rendezvous::verify::{MAX_SEEN, SessionGuard, verify_offer};
use crate::runtime_identity::RuntimeIdentity;

/// Shown when a host has gathered no candidate that could work from outside the
/// house. Hosting still starts — the LAN path is real and useful — but saying
/// nothing would leave the player wondering why their friend never arrives.
pub fn unreachable_warning() -> &'static str {
    "Friends outside your home probably can't reach you. Turn on UPnP on your router."
}

/// The whole set of personas a `HostedServer` should admit while hosting
/// online: the host's own persona, the player's own close contacts, and
/// everybody this session's bearer has actually let in.
///
/// `HostedServer::set_access_policy` **replaces** its whitelist, so pushing
/// [`OnlineHost::allowlist`] alone would silently revoke every contact who
/// hasn't called yet. All three parts go in together, deduplicated, every time.
///
/// This list is the only gate on the bound QUIC socket: the relay offer gate
/// doesn't stand in front of it, and anybody who finds the port can dial it
/// directly. So [`OnlineHost::access_policy`] always hands it over as
/// [`Allow::Only`], never [`Allow::Open`], however short it is: a first-ever
/// host with no contacts and nobody admitted admits itself and no stranger.
///
/// Every pubkey in `blocked` (Signet blocks, D8) is left out of both halves —
/// a block beats a contact row and a this-session admission alike. The same list goes to `HostedServer` as its blocklist.
pub fn access_allowlist(
    host_persona: &[u8; 32],
    book: &[Contact],
    admitted: &[[u8; 32]],
    blocked: &[[u8; 32]],
) -> Vec<[u8; 32]> {
    let mut out: Vec<[u8; 32]> = Vec::with_capacity(book.len() + admitted.len() + 1);
    out.push(*host_persona);
    for pk in book
        .iter()
        .filter(|c| crate::online_admission::admits_play(c.tier))
        .map(|c| c.pubkey)
        .chain(admitted.iter().copied())
    {
        if !out.contains(&pk) && !blocked.contains(&pk) {
            out.push(pk);
        }
    }
    out
}

/// A `start` that failed, and the relay it was handed.
///
/// Dropping a `MultiRelay` waits up to half a second for its worker to put its
/// sockets down. On the success path the relay lives inside the returned value
/// and is retired off-thread when hosting stops; on the failure path it used to
/// be dropped right there, inline, on the frame that pressed the button — half
/// a second of frozen window for a session that never started. Handing it back
/// lets the caller retire it off the frame like every other one.
pub type StartFailed = (String, Box<dyn RendezvousRelay>);

/// How many relay events one [`OnlineHost::poll`] will handle.
///
/// `poll` runs on the game-loop thread, every frame, and each event costs a
/// signature verification and a NIP-44 decrypt (an admitted one also spawns a
/// punch thread). The relay channel is fed by the network and has no bound of
/// its own, so a flood is spread across frames rather than allowed to stall
/// one. At 20 TPS this still clears 320 events a second — far more than a
/// household host will ever see honestly.
pub const MAX_EVENTS_PER_POLL: usize = 16;

/// How many punch workers one host may have running at once.
///
/// Every admitted offer spawns a thread that sleeps ~200 ms sending datagrams.
/// Uncapped, a burst of admitted offers — a friend retrying, or one contact
/// whose relays all deliver a fresh session — is a thread per offer, all at
/// once. Four is more than a household host ever needs concurrently, and an
/// offer that arrives while all four are busy is simply not read this poll: it
/// stays in the relay's queue and is handled on the next one.
pub const MAX_PUNCH_THREADS: usize = 4;

/// The in-flight punch-worker count, handed out as permits.
///
/// A permit gives its slot back when it is dropped, which happens when the
/// worker thread ends — so the count cannot drift even if a punch panics.
#[derive(Clone, Default)]
struct PunchSlots(Arc<AtomicUsize>);

/// One permit. Dropping it frees the slot.
struct PunchSlot(Arc<AtomicUsize>);

impl PunchSlots {
    /// A permit, or `None` when [`MAX_PUNCH_THREADS`] are already out.
    fn try_acquire(&self) -> Option<PunchSlot> {
        let mut current = self.0.load(Ordering::Relaxed);
        loop {
            if current >= MAX_PUNCH_THREADS {
                return None;
            }
            match self.0.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(PunchSlot(Arc::clone(&self.0))),
                Err(actual) => current = actual,
            }
        }
    }

    fn in_flight(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
}

impl Drop for PunchSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Something the game loop needs to act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEvent {
    Admitted {
        persona: [u8; 32],
        reason: AdmitReason,
    },
    /// Push this into `HostedServer::set_access_policy`.
    AllowlistChanged(Vec<[u8; 32]>),
    /// An offer was discarded. Carries a reason for the log only — nothing was
    /// sent back.
    Dropped(String),
}

/// The event ids already handled, so the same offer delivered by four relays is
/// verified (and answered) exactly once.
///
/// `SessionGuard` is not a substitute: it is keyed on the session inside the
/// ciphertext, so reaching it costs a signature verification and a NIP-44
/// decrypt per copy — work this does on the game loop's thread. Bounded by
/// [`MAX_SEEN`], oldest-first, for the same reason `SessionGuard` is.
#[derive(Default)]
struct SeenEvents {
    ids: HashSet<EventId>,
    order: VecDeque<EventId>,
}

impl SeenEvents {
    /// `true` if this id is new (and is now remembered).
    fn check_and_insert(&mut self, id: EventId) -> bool {
        if !self.ids.insert(id) {
            return false;
        }
        self.order.push_back(id);
        while self.order.len() > MAX_SEEN {
            if let Some(oldest) = self.order.pop_front() {
                self.ids.remove(&oldest);
            }
        }
        true
    }
}

pub struct OnlineHost {
    identity: RuntimeIdentity,
    /// The host's own persona, x-only. The invite carries its npub form; this
    /// is kept for the Online panel (Task 17), which shows who is hosting.
    persona: [u8; 32],
    relay: Box<dyn RendezvousRelay>,
    /// A `try_clone` of the socket quinn owns, so punches leave from the same
    /// source port the peer was told to expect. Shared with the punch worker.
    punch_socket: Arc<UdpSocket>,
    candidates: Vec<Candidate>,
    world_name: String,
    relays: Vec<String>,
    book: Vec<Contact>,
    allowlist: Vec<[u8; 32]>,
    /// Who holds which bearer. See [`BearerClaim`]: emptied when a bearer may
    /// have left this machine ([`OnlineHost::rotate_bearer`]).
    claims: Vec<BearerClaim>,
    /// Everybody a bearer has let in this session. They stay on the allowlist
    /// through a book push until blocked, and this is all that holds them there:
    /// an invite writes no row, so nothing about them outlives this host.
    guests: Vec<[u8; 32]>,
    /// Signet-blocked pubkeys (D8). Refused before any other check, kept off
    /// the allowlist, and handed to `HostedServer` as its blocklist.
    blocked: Vec<[u8; 32]>,
    invite: Invite,
    active_bearer: ActiveBearer,
    guard: SessionGuard,
    seen: SeenEvents,
    punch_slots: PunchSlots,
    continuation: Option<crate::handover::live::continuation::Resolved>,
    retired_capabilities: Vec<crate::handover::phase::RetiredRoot>,
    continuation_revoked: bool,
}

impl OnlineHost {
    /// Mint the first invite and subscribe for offers.
    ///
    /// `punch_socket` should be a `try_clone()` of the socket handed to
    /// `HostedServer::start_online`; `candidates` are what
    /// `nat::candidates::gather` found on it.
    #[allow(clippy::too_many_arguments)] // every one is a distinct dependency
    pub fn start(
        identity: RuntimeIdentity,
        persona: [u8; 32],
        relay: Box<dyn RendezvousRelay>,
        punch_socket: UdpSocket,
        candidates: Vec<Candidate>,
        world_name: String,
        relays: Vec<String>,
        book: Vec<Contact>,
        now: u64,
    ) -> Result<OnlineHost, StartFailed> {
        let runtime_pk = identity.runtime_pubkey();
        // Every early return hands the relay BACK rather than dropping it here
        // — see [`StartFailed`].
        if let Err(e) = relay.subscribe(KIND_JOIN_OFFER, &runtime_pk.to_hex()) {
            return Err((e, relay));
        }
        let host_persona = match nostr::PublicKey::from_slice(&persona) {
            Ok(pk) => npub_of(&pk),
            Err(e) => return Err((e.to_string(), relay)),
        };

        let bearer = mint_bearer();
        let expires_at = now + DEFAULT_INVITE_TTL_SECS;
        let invite = Invite {
            host_persona,
            host_runtime: runtime_pk.to_bytes(),
            relays: relays.clone(),
            bearer,
            expires_at,
            world_name: world_name.clone(),
        };

        Ok(OnlineHost {
            identity,
            persona,
            relay,
            punch_socket: Arc::new(punch_socket),
            candidates,
            world_name,
            relays,
            book,
            allowlist: Vec::new(),
            claims: Vec::new(),
            guests: Vec::new(),
            blocked: Vec::new(),
            invite,
            active_bearer: ActiveBearer { bearer, expires_at },
            guard: SessionGuard::new(),
            seen: SeenEvents::default(),
            punch_slots: PunchSlots::default(),
            continuation: None,
            retired_capabilities: Vec::new(),
            continuation_revoked: false,
        })
    }

    /// The invite as minted, for a test that needs its fields. Production goes
    /// through [`OnlineHost::invite_link`].
    #[cfg(test)]
    pub fn invite(&self) -> &Invite {
        &self.invite
    }

    /// The persona this machine is hosting as. Always on the access allowlist —
    /// see [`access_allowlist`].
    #[cfg_attr(not(test), allow(dead_code))] // production reads it via `access_policy`
    pub fn persona(&self) -> &[u8; 32] {
        &self.persona
    }

    pub fn invite_link(&self, now: u64) -> Result<String, &'static str> {
        if self.continuation.is_some() && self.live_bearer(now).is_none() {
            return Err("The original invite is unavailable. Contacts can still join directly.");
        }
        Ok(self.invite.to_link())
    }

    pub(crate) fn install_continuation(
        &mut self,
        resolved: crate::handover::live::continuation::Resolved,
        now: u64,
    ) -> Result<(), String> {
        let authority = resolved.authority().map_err(|e| e.to_string())?;
        if authority.runtime() != self.identity.runtime_pubkey().to_bytes()
            || authority.persona() != self.persona
            || now > resolved.expiry()
        {
            return Err("This device isn't the verified host of the original invitation.".into());
        }
        self.invite.host_runtime = resolved.root.runtime();
        self.invite.host_persona = npub_of(
            &nostr::PublicKey::from_slice(&resolved.root.persona()).map_err(|e| e.to_string())?,
        );
        self.invite.relays = resolved.root.relays().to_vec();
        self.invite.expires_at = resolved.root.expiry();
        self.invite.bearer = *resolved.bearer.expose();
        self.active_bearer = ActiveBearer {
            bearer: self.invite.bearer,
            expires_at: resolved.expiry(),
        };
        self.continuation = Some(resolved);
        Ok(())
    }

    pub(crate) fn continuation(&self) -> Option<&crate::handover::live::continuation::Resolved> {
        self.continuation.as_ref()
    }

    pub(crate) fn install_retained(&mut self, world: &std::path::Path) -> Result<(), String> {
        if let Some(info) =
            crate::handover::live::retained::read(world).map_err(|e| e.to_string())?
        {
            if info.core().source_persona != self.persona {
                return Err(
                    "Sign in as this retained world's saved host before hosting it.".into(),
                );
            }
            self.retired_capabilities = info.retirement.retired_roots().to_vec();
        }
        Ok(())
    }

    fn handover_context(&self, now: u64) -> Option<crate::rendezvous::payload::HandoverContext> {
        if let Some(resolved) = &self.continuation {
            if self.continuation_revoked
                || now > resolved.expiry()
                || self.active_bearer.bearer != *resolved.bearer.expose()
            {
                return None;
            }
            return resolved.admission().ok();
        }
        let root = self.root_context(now).ok()?;
        Some(crate::rendezvous::payload::HandoverContext {
            root: root.digest(),
            capability: crate::handover::crypto::root_capability(&root, &self.active_bearer.bearer)
                .ok()?,
            epoch: 0,
            committed: [0; 32],
        })
    }

    /// The live invitation's root fields (its `k`, `h`, `x` and `r`), the
    /// context a world handover from this host is bound to (Spec 04 "World
    /// handover"). The bearer is not part of it.
    pub fn root_context(
        &self,
        now: u64,
    ) -> crate::handover::Result<crate::handover::context::RootContext> {
        if let Some(resolved) = &self.continuation {
            if self.continuation_revoked
                || now > resolved.expiry()
                || self.active_bearer.bearer != *resolved.bearer.expose()
            {
                return Err(crate::handover::Error::Invalid(
                    "original capability is unavailable",
                ));
            }
            return Ok(resolved.root.clone());
        }
        crate::handover::context::RootContext::new(
            self.invite.host_runtime,
            &self.invite.host_persona,
            self.invite.expires_at,
            &self.invite.relays,
            now,
        )
    }

    /// The runtime key this host answers and signs with.
    pub fn runtime_keys(&self) -> &nostr::Keys {
        self.identity.keys()
    }

    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    pub fn relays_connected(&self) -> (usize, usize) {
        self.relay.connected()
    }

    /// Setup events the relay layer threw away because this host was behind.
    /// Shown on the Online panel — a dropped offer is a friend who could not
    /// get in, and that must not be silent.
    pub fn relays_dropped(&self) -> usize {
        self.relay.dropped()
    }

    /// The personas admitted through the rendezvous **this session**. It grows
    /// as offers are accepted and starts empty — it is NOT the whole access
    /// policy on its own. Pair it with the contacts book through
    /// [`access_allowlist`].
    #[cfg_attr(not(test), allow(dead_code))] // production reads it via `access_policy`
    pub fn allowlist(&self) -> &[[u8; 32]] {
        &self.allowlist
    }

    /// Replace the contacts snapshot this host admits from.
    ///
    /// The book handed to [`OnlineHost::start`] is a snapshot (see the module
    /// docs). A contact added, re-tiered or removed while a world is hosted only
    /// takes effect once the owner of the mirror pushes the fresh book in here —
    /// the game loop does that whenever it writes the mirror.
    ///
    /// Somebody a bearer let in stays for this session until they are blocked,
    /// whatever the book says. Anyone else this session admitted must still be
    /// close enough in the new book, or they leave the session's admissions too.
    /// `blocked` beats every source (D8), so a blocked pubkey leaves the book,
    /// this session's admissions and (through [`OnlineHost::access_policy`]) the
    /// server's whitelist. A block keeps their claim: it must not free their
    /// bearer for somebody else.
    pub fn set_book(&mut self, book: Vec<Contact>, blocked: Vec<[u8; 32]>) {
        self.book = book;
        self.book.retain(|c| !blocked.contains(&c.pubkey));
        let new_book = &self.book;
        let guests = &self.guests;
        self.allowlist.retain(|pk| {
            !blocked.contains(pk)
                && (guests.contains(pk)
                    || crate::online_admission::admits_play(crate::contacts::tier_of(new_book, pk)))
        });
        self.blocked = blocked;
    }

    /// `(allowlist, blocklist)` for `HostedServer::set_access_policy`: the
    /// host, its close contacts and this session's admissions, minus every
    /// block — and the blocks themselves. Every push goes through here.
    pub fn access_policy(&self) -> (Allow, Vec<[u8; 32]>) {
        (
            Allow::only(access_allowlist(&self.persona, &self.book, &self.allowlist, &self.blocked)),
            self.blocked.clone(),
        )
    }

    /// The book this host is admitting from, for the caller that owns the
    /// access policy.
    pub fn book(&self) -> &[Contact] {
        &self.book
    }

    /// H1: the live invitation's bearer (the one `admit` checks as
    /// `Accept(ByInvite)`), or `None` once it has expired: a world handover
    /// needs a live invitation capability and never mints its own kind.
    pub fn live_bearer(&self, now: u64) -> Option<[u8; crate::invite::BEARER_LEN]> {
        (!self.continuation_revoked && self.active_bearer.expires_at >= now)
            .then_some(self.active_bearer.bearer)
    }

    /// A root host replaces its disclosed bearer. A continued authority
    /// preserves the original fields and makes that capability unavailable;
    /// its world-local abort record prevents resurrection on restart.
    ///
    /// The bearer may have left this machine, so its owner loses it as well: no
    /// grace for anybody. A seated guest stays on this session's list.
    pub fn rotate_bearer(&mut self) -> &Invite {
        self.claims.clear();
        // This authority inherited one immutable original capability. It
        // cannot replace that bearer's value under the original host's key.
        if self.continuation.is_some() {
            self.continuation_revoked = true;
            return &self.invite;
        }
        let bearer = mint_bearer();
        self.invite.bearer = bearer;
        self.active_bearer.bearer = bearer;
        &self.invite
    }

    /// Test builds: what the live invitation decides for `persona` joining
    /// with the invite string `link` (the same `admit` the offer path runs).
    #[cfg(test)]
    pub fn admission_for_link(
        &self,
        persona: &[u8; 32],
        link: &str,
        now: u64,
    ) -> crate::online_admission::Admission {
        let offered = crate::invite::Invite::parse(link, now)
            .ok()
            .map(|i| i.bearer);
        admit(
            persona,
            offered,
            &self.book,
            (!self.continuation_revoked).then_some(&self.active_bearer),
            &self.claims,
            now,
        )
    }

    /// Whether the current link has already let somebody in, so it would do
    /// nothing for the next person it is sent to.
    pub fn link_is_used(&self) -> bool {
        self.claims.iter().any(|c| bearer_eq(&c.bearer, &self.active_bearer.bearer))
    }

    /// Copy a continued original invitation unchanged, or mint a fresh
    /// invitation for an ordinary root host. The previous bearer lets nobody new
    /// in; if it has an owner, they keep it until it expires.
    pub fn mint_fresh_invite(&mut self, now: u64) -> Result<&Invite, &'static str> {
        if self.continuation.is_some() {
            if self.live_bearer(now).is_none() {
                return Err(
                    "The original invite is unavailable. Contacts can still join directly.",
                );
            }
            return Ok(&self.invite);
        }
        let bearer = mint_bearer();
        let expires_at = now + DEFAULT_INVITE_TTL_SECS;
        self.invite = Invite {
            host_persona: self.invite.host_persona.clone(),
            host_runtime: self.invite.host_runtime,
            relays: self.relays.clone(),
            bearer,
            expires_at,
            world_name: self.world_name.clone(),
        };
        self.active_bearer = ActiveBearer { bearer, expires_at };
        Ok(&self.invite)
    }

    /// Drain and answer what the relays have delivered, at most
    /// [`MAX_EVENTS_PER_POLL`] events per call. Non-blocking.
    ///
    /// The order of the gates is the spec's, not the cheapest one: **who is
    /// asking is decided first, in silence**, and only somebody already
    /// entitled to reach this host is ever told anything (spec §3.3). Deciding
    /// the protocol or capacity refusal first would answer a stranger who sent
    /// a wrong version number — and that answer carries the host's persona,
    /// attestation and world name.
    pub fn poll(&mut self, now: u64, players: usize, capacity: usize) -> Vec<HostEvent> {
        let mut out = Vec::new();
        for _ in 0..MAX_EVENTS_PER_POLL {
            // An admitted offer costs a punch worker, and only
            // [`MAX_PUNCH_THREADS`] may be out at once. Stop READING rather
            // than admit somebody we cannot punch for: the offer stays in the
            // relay's queue and the next poll picks it up.
            if self.punch_slots.in_flight() >= MAX_PUNCH_THREADS {
                break;
            }
            let Some(ev) = self.relay.try_recv() else {
                break;
            };
            // The id is a hash of the event's own fields, and it is about to be
            // used as a dedupe key — so check it before trusting it. An event
            // carrying a genuine offer's id would otherwise pre-empt the real
            // one and be silently swallowed by the dedupe set.
            if !ev.verify_id() {
                out.push(HostEvent::Dropped("forged event id".to_string()));
                continue;
            }
            // A four-relay set hands back four copies of one offer. Drop the
            // repeats before spending a signature check on them.
            if !self.seen.check_and_insert(ev.id) {
                continue;
            }
            let verified = match verify_offer(&ev, self.identity.keys(), &mut self.guard, now) {
                Ok(v) => v,
                Err(e) => {
                    // Silence. The variant is for the log, not for the caller.
                    out.push(HostEvent::Dropped(format!("{e:?}")));
                    continue;
                }
            };

            // WHO, first, and in silence. A Signet block beats a contact row
            // and a live bearer alike (D8) — no answer, no allowlist entry.
            if self.blocked.contains(&verified.persona) {
                out.push(HostEvent::Dropped("refused: blocked in Signet".to_string()));
                continue;
            }
            let bearer = verified
                .offer
                .bearer
                .as_deref()
                .and_then(|h| hex::decode(h).ok())
                .and_then(|b| <[u8; 16]>::try_from(b.as_slice()).ok());
            // The bearer this persona already owns, if that is what they hold. It
            // may no longer be the live one, so it is checked against, and
            // answered in, the context it was claimed in. Nobody else gets past
            // these checks with a replaced bearer.
            let own_claim = bearer.and_then(|b| {
                self.claims
                    .iter()
                    .find(|c| c.owner == verified.persona && bearer_eq(&c.bearer, &b))
                    .copied()
            });
            let context = match own_claim {
                Some(claim) => claim.context,
                None => self.handover_context(now),
            };
            // Check continued/retained authority before Kith/contact
            // shortcuts, including a runtime returning at a later epoch.
            if (self.continuation_revoked
                && (verified.offer.bearer.is_some() || verified.offer.handover.is_some()))
                || verified.offer.handover.is_some_and(|expected| Some(expected) != context)
                || ((self.continuation.is_some() || !self.retired_capabilities.is_empty())
                    && verified.offer.bearer.is_some()
                    && verified.offer.handover.is_none())
                || (verified.offer.handover.is_some()
                    && own_claim.is_none()
                    && bearer != Some(self.active_bearer.bearer))
                || verified.offer.handover.is_some_and(|expected| {
                    self.retired_capabilities
                        .iter()
                        .any(|root| root.capability == expected.capability)
                })
            {
                out.push(HostEvent::Dropped(
                    "refused: original invitation authority differs".into(),
                ));
                continue;
            }
            let reason = match admit(
                &verified.persona,
                bearer,
                &self.book,
                Some(&self.active_bearer),
                &self.claims,
                now,
            ) {
                Admission::Accept(reason) => reason,
                Admission::Refuse(r) => {
                    out.push(HostEvent::Dropped(format!("refused: {r:?}")));
                    // No identity refusal is reply-worthy, so this sends
                    // nothing today; it is written as a gate rather than a
                    // `continue` so a future refusal kind cannot be silently
                    // swallowed here.
                    if is_reply_worthy(r) {
                        self.send_answer(
                            &verified.runtime,
                            &verified.offer.session,
                            false,
                            Some(refusal_wire(r)),
                            context,
                            now,
                        );
                    }
                    continue;
                }
            };

            // Only now — to somebody who was entitled to reach this host — are
            // the two refusals we explain in play.
            let explained = if verified.offer.protocol != crate::protocol::PROTOCOL_VERSION {
                Some(Refusal::ProtocolMismatch)
            } else if players >= capacity {
                Some(Refusal::Full)
            } else {
                None
            };
            if let Some(r) = explained {
                out.push(HostEvent::Dropped(format!("refused: {r:?}")));
                if is_reply_worthy(r) {
                    self.send_answer(
                        &verified.runtime,
                        &verified.offer.session,
                        false,
                        Some(refusal_wire(r)),
                        context,
                        now,
                    );
                }
                // Nothing is claimed by a refusal, an invitee's included: they
                // never got in, and the bearer is still free for the retry that
                // follows the upgrade. Being told "your build is old" is the
                // whole reply.
                continue;
            }

            // This hosted world's session only: a claim and an allowlist entry,
            // no contact row, on disk or in the book.
            if reason == AdmitReason::ByInvite {
                if own_claim.is_none() {
                    self.claims.push(BearerClaim {
                        bearer: self.active_bearer.bearer,
                        owner: verified.persona,
                        expires_at: self.active_bearer.expires_at,
                        context,
                    });
                }
                if !self.guests.contains(&verified.persona) {
                    self.guests.push(verified.persona);
                }
            }
            if !self.allowlist.contains(&verified.persona) {
                self.allowlist.push(verified.persona);
                out.push(HostEvent::AllowlistChanged(self.allowlist.clone()));
            }
            out.push(HostEvent::Admitted {
                persona: verified.persona,
                reason,
            });

            // Punch toward the joiner BEFORE answering, so by the time they
            // start dialling, this router already expects them.
            self.punch(&verified.offer.candidates, &verified.offer.session);
            self.send_answer(&verified.runtime, &verified.offer.session, true, None, context, now);
        }
        out
    }

    /// Fire punches at the joiner's candidates on a worker thread — the send
    /// loop sleeps ~200 ms and must never do that on the game loop.
    fn punch(&self, candidates: &[Candidate], session: &str) {
        let targets = parse_addrs(candidates);
        if targets.is_empty() {
            return;
        }
        // Held by the worker and released when its thread ends.
        let Some(slot) = self.punch_slots.try_acquire() else {
            log::debug!("[online] {MAX_PUNCH_THREADS} punches already in flight; skipping one");
            return;
        };
        let sock = Arc::clone(&self.punch_socket);
        let session = session.to_string();
        if let Err(e) = std::thread::Builder::new()
            .name("online-punch".into())
            .spawn(move || {
                crate::nat::punch::send_punches(&sock, &targets, &session);
                drop(slot);
            })
        {
            log::warn!("[online] could not spawn the punch thread: {e}");
        }
    }

    fn send_answer(
        &self,
        to_runtime: &[u8; 32],
        session: &str,
        accepted: bool,
        reason: Option<&str>,
        handover: Option<crate::rendezvous::payload::HandoverContext>,
        now: u64,
    ) {
        let Some(attestation) = self.identity.attestation().cloned() else {
            log::warn!("[online] cannot answer without an attestation");
            return;
        };
        let Ok(recipient) = nostr::PublicKey::from_slice(to_runtime) else {
            return;
        };
        let answer = Answer {
            v: PAYLOAD_VERSION,
            session: session.to_string(),
            persona: npub_of(
                &nostr::PublicKey::from_slice(&self.persona).expect("validated host persona"),
            ),
            attestation,
            accepted,
            reason: reason.map(str::to_string),
            protocol: crate::protocol::PROTOCOL_VERSION,
            // A refusal names no addresses — there is no reason to hand them to
            // somebody who is not coming in.
            candidates: if accepted {
                self.candidates.clone()
            } else {
                Vec::new()
            },
            world_name: self.world_name.clone(),
            sent_at: now,
            handover,
        };
        let keys = self.identity.keys().clone();
        // Sealing is pure CPU (a NIP-44 encrypt and a signature) and only
        // `async` because `NostrSigner` is; nothing here awaits I/O.
        // `pollster`, not a tokio runtime: `Runtime::block_on` panics when the
        // caller is already inside a runtime, which the integration tests (and
        // any future async driver) are.
        match pollster::block_on(seal_answer(&keys, &recipient, &answer)) {
            Ok(ev) => {
                debug_assert_eq!(ev.kind, nostr::Kind::Custom(KIND_JOIN_ANSWER));
                if let Err(e) = self.relay.publish(&ev) {
                    log::warn!("[online] could not publish the answer: {e}");
                }
            }
            Err(e) => log::warn!("[online] could not seal the answer: {e}"),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::contacts::AddedVia;
    use crate::rendezvous::payload::{Offer, new_session_id, seal_offer};
    use crate::rendezvous::relay_client::{FakeRelayHub, RendezvousRelay};
    use nostr::Keys;

    const NOW: u64 = 1_700_000_000;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    struct Caller {
        persona: Keys,
        runtime: Keys,
    }

    impl Caller {
        fn new() -> Self {
            Caller {
                persona: Keys::generate(),
                runtime: Keys::generate(),
            }
        }
        async fn offer(&self, bearer: Option<String>, protocol: u32) -> Offer {
            Offer {
                v: PAYLOAD_VERSION,
                session: new_session_id(),
                persona: npub_of(&self.persona.public_key()),
                attestation: crate::runtime_identity::mint_player_attestation(
                    &self.persona,
                    &self.runtime.public_key(),
                    nostr::Timestamp::from(NOW - 10),
                    90,
                )
                .await
                .unwrap(),
                bearer,
                protocol,
                candidates: vec![Candidate {
                    kind: "lan".to_string(),
                    addr: "127.0.0.1:9".to_string(),
                }],
                sent_at: NOW,
                handover: None,
            }
        }
    }

    /// A host wired to a fake relay, with `book` as its contacts.
    pub(crate) fn a_host(hub: &FakeRelayHub, book: Vec<Contact>) -> (OnlineHost, Keys) {
        let host_persona = Keys::generate();
        let host_runtime = Keys::generate();
        // `pollster`, not `rt()`: most callers are already inside
        // `rt().block_on(async { … })`, and a nested tokio runtime panics.
        let attestation = pollster::block_on(crate::runtime_identity::mint_player_attestation(
            &host_persona,
            &host_runtime.public_key(),
            nostr::Timestamp::from(NOW - 10),
            90,
        ))
        .unwrap();
        let identity = RuntimeIdentity::from_parts(host_runtime.clone(), Some(attestation));
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let host = OnlineHost::start(
            identity,
            host_persona.public_key().to_bytes(),
            Box::new(hub.client()),
            sock,
            vec![Candidate {
                kind: "lan".to_string(),
                addr: "127.0.0.1:7700".to_string(),
            }],
            "Ivy's Hollow".to_string(),
            vec!["wss://nos.lol".to_string()],
            book,
            NOW,
        )
        .map_err(|(e, _relay)| e)
        .unwrap();
        (host, host_runtime)
    }

    pub(crate) fn a_contact(pk: [u8; 32], tier: crate::comms::Tier) -> Contact {
        Contact {
            pubkey: pk,
            display_name: Some("Friend".to_string()),
            tier,
            is_child: false,
            runtime_pubkey: None,
            added_via: AddedVia::Kenspeckle,
            added_at: 0,
            last_joined: None,
        }
    }

    #[test]
    fn the_access_list_is_the_close_contacts_plus_this_session_s_admissions() {
        // set_access_policy REPLACES the whitelist, so the two halves must be
        // pushed together — a list of only the admitted would revoke every
        // contact who hasn't called yet.
        let book = vec![
            a_contact([1; 32], crate::comms::Tier::Kin),
            a_contact([2; 32], crate::comms::Tier::Kith),
            a_contact([3; 32], crate::comms::Tier::Ken),
        ];
        const ME: [u8; 32] = [42; 32];
        let list = access_allowlist(&ME, &book, &[[9; 32], [1; 32]], &[]);
        assert!(list.contains(&[1; 32]) && list.contains(&[2; 32]));
        assert!(
            !list.contains(&[3; 32]),
            "ken may be recognised but may not play"
        );
        assert!(list.contains(&[9; 32]), "a bearer admission must survive");
        assert!(list.contains(&ME), "the host is always on its own list");
        assert_eq!(
            list.len(),
            4,
            "no duplicate for the contact who also called: {list:?}"
        );
    }

    /// REGRESSION (whole-branch review, MINOR 11). One punch worker was spawned
    /// per admitted offer with nothing bounding them, so a burst of admitted
    /// offers was a thread apiece, all sleeping ~200 ms at once.
    #[test]
    fn at_most_four_punch_workers_are_ever_out_at_once() {
        let slots = PunchSlots::default();
        let mut held: Vec<PunchSlot> = (0..MAX_PUNCH_THREADS)
            .map(|_| slots.try_acquire().expect("the first four are free"))
            .collect();
        assert_eq!(slots.in_flight(), MAX_PUNCH_THREADS);
        assert!(slots.try_acquire().is_none(), "the fifth must wait");
        assert_eq!(MAX_PUNCH_THREADS, 4, "the cap the review asked for");

        // A worker finishing frees exactly one slot, and no more. (`pop` drops
        // the one it returns and leaves the rest held — `into_iter().next()`
        // would drop the whole vector with the iterator.)
        held.pop();
        assert_eq!(slots.in_flight(), MAX_PUNCH_THREADS - 1);
        let one = slots.try_acquire().expect("the freed slot is reusable");
        assert!(slots.try_acquire().is_none(), "and only the one");
        drop(one);
        drop(held);
        assert_eq!(slots.in_flight(), 0, "every worker gives its slot back");
    }

    #[test]
    fn every_permit_gives_its_slot_back() {
        let slots = PunchSlots::default();
        for _ in 0..100 {
            let s = slots.try_acquire().expect("nothing should ever leak");
            drop(s);
        }
        assert_eq!(slots.in_flight(), 0);
    }

    #[test]
    fn a_host_that_is_already_a_contact_appears_once() {
        const ME: [u8; 32] = [1; 32];
        let book = vec![a_contact(ME, crate::comms::Tier::Kin)];
        let list = access_allowlist(&ME, &book, &[ME], &[]);
        assert_eq!(list, vec![ME], "one row, however many ways it got there");
    }

    /// REGRESSION (whole-branch review, CRITICAL 1). A first-ever online host
    /// has no contacts and has admitted nobody, and its QUIC socket can be
    /// dialled directly, past the relay offer gate. Its policy is an allowlist
    /// all the same, so a signed-in stranger who finds the port is refused.
    #[test]
    fn a_fresh_online_host_still_refuses_a_signed_in_stranger() {
        const STRANGER: [u8; 32] = [7; 32];
        let hub = FakeRelayHub::new();
        let (host, _) = a_host(&hub, vec![]);
        let (allow, _) = host.access_policy();
        assert!(matches!(allow, Allow::Only(_)), "never an open server: {allow:?}");
        assert_eq!(
            crate::access_policy::decide_access(Some(STRANGER), &[], &allow, true),
            Err(crate::access_policy::AccessReject::NotAllowlisted),
        );
        // …and the host itself is still allowed onto its own world.
        assert!(crate::access_policy::decide_access(Some(*host.persona()), &[], &allow, true).is_ok());
    }

    #[test]
    fn a_refreshed_book_replaces_the_snapshot() {
        // The book is a start-time snapshot; the game loop pushes a fresh one
        // whenever it writes the mirror.
        let hub = FakeRelayHub::new();
        let (mut host, _) = a_host(&hub, vec![a_contact([7; 32], crate::comms::Tier::Kith)]);
        host.set_book(
            vec![a_contact([1; 32], crate::comms::Tier::Kin)],
            Vec::new(),
        );
        assert!(
            crate::contacts::find(host.book(), &[1; 32]).is_some(),
            "the fresh book is in"
        );
        assert!(
            crate::contacts::find(host.book(), &[7; 32]).is_none(),
            "and the old one is gone"
        );
    }

    #[test]
    fn the_minted_invite_round_trips_and_names_this_host() {
        let hub = FakeRelayHub::new();
        let (host, host_runtime) = a_host(&hub, vec![]);
        let parsed = crate::invite::Invite::parse(&host.invite_link(NOW).unwrap(), NOW).unwrap();
        assert_eq!(parsed.host_runtime, host_runtime.public_key().to_bytes());
        assert_eq!(parsed.world_name, "Ivy's Hollow");
        assert_eq!(parsed.relays, vec!["wss://nos.lol".to_string()]);
        assert_eq!(
            parsed.expires_at,
            NOW + crate::invite::DEFAULT_INVITE_TTL_SECS
        );
    }

    #[test]
    fn rotating_the_bearer_keeps_the_root_context_and_retires_the_old_link() {
        let hub = FakeRelayHub::new();
        let (mut host, _) = a_host(&hub, vec![]);
        let (old_link, root) = (
            host.invite_link(NOW).unwrap(),
            host.root_context(NOW).unwrap().digest(),
        );
        let (old, expiry, relays) = (
            host.invite().bearer,
            host.invite().expires_at,
            host.invite().relays.clone(),
        );
        let new = host.rotate_bearer().bearer;
        assert_ne!(old, new);
        assert_eq!(
            (host.invite().expires_at, &host.invite().relays),
            (expiry, &relays)
        );
        assert_eq!(
            host.root_context(NOW + 60).unwrap().digest(),
            root,
            "rotation changed the root"
        );
        assert_eq!(host.live_bearer(NOW + 60), Some(new));
        let stranger = [0x44; 32];
        assert!(matches!(
            host.admission_for_link(&stranger, &old_link, NOW + 60),
            Admission::Refuse(_)
        ));
        assert!(matches!(
            host.admission_for_link(&stranger, &host.invite_link(NOW).unwrap(), NOW + 60),
            Admission::Accept(_)
        ));
    }

    #[test]
    fn minting_a_fresh_invite_retires_the_old_bearer() {
        let hub = FakeRelayHub::new();
        let (mut host, _) = a_host(&hub, vec![]);
        let old = host.invite().bearer;
        let new = host.mint_fresh_invite(NOW + 60).unwrap().bearer;
        assert_ne!(old, new);
    }

    #[test]
    fn a_kith_contact_is_admitted_and_lands_on_the_allowlist() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(
                &hub,
                vec![a_contact(
                    caller.persona.public_key().to_bytes(),
                    crate::comms::Tier::Kith,
                )],
            );
            let joiner_relay = hub.client();
            let offer = caller.offer(None, crate::protocol::PROTOCOL_VERSION).await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();

            let events = host.poll(NOW, 1, 5);
            assert!(
                events.iter().any(|e| matches!(
                    e,
                    HostEvent::Admitted {
                        reason: AdmitReason::AlreadyContact,
                        ..
                    }
                )),
                "{events:?}"
            );
            assert!(
                host.allowlist()
                    .contains(&caller.persona.public_key().to_bytes())
            );
        });
    }

    #[test]
    fn a_contact_who_is_no_longer_close_leaves_this_sessions_admissions() {
        use crate::comms::Tier;
        let hub = FakeRelayHub::new();
        let book = vec![
            a_contact([1; 32], Tier::Kith),
            a_contact([2; 32], Tier::Kith),
            a_contact([3; 32], Tier::Kith),
        ];
        let (mut host, _) = a_host(&hub, book);
        host.allowlist.extend([[1; 32], [2; 32], [3; 32]]);

        // [1] is gone from the book, [2] is downgraded, [3] is unchanged.
        host.set_book(vec![a_contact([2; 32], Tier::Ken), a_contact([3; 32], Tier::Kith)], vec![]);

        assert_eq!(host.allowlist(), &[[3; 32]], "only a contact still close enough stays");
        let (whitelist, _) = host.access_policy();
        assert!(!whitelist.admits(&[1; 32]) && !whitelist.admits(&[2; 32]), "{whitelist:?}");
        assert!(whitelist.admits(&[3; 32]));
    }

    #[test]
    fn a_signet_block_removes_a_guest_and_an_admission_and_is_the_blocklist() {
        // D8: a block beats every source — including this session's guests and
        // admissions, which `set_book` and `access_allowlist` otherwise keep on
        // purpose.
        let hub = FakeRelayHub::new();
        let (mut host, _) = a_host(&hub, vec![]);
        host.guests.push([7; 32]);
        host.allowlist.push([9; 32]);
        host.allowlist.push([7; 32]);
        let fresh = vec![
            a_contact([1; 32], crate::comms::Tier::Kin),
            a_contact([9; 32], crate::comms::Tier::Kin),
        ];
        host.set_book(fresh, vec![[7; 32], [9; 32]]);

        assert!(
            crate::contacts::find(host.book(), &[9; 32]).is_none(),
            "blocked contact left the book"
        );
        assert!(
            host.allowlist().is_empty(),
            "blocked admissions dropped: {:?}",
            host.allowlist()
        );
        let (whitelist, blocklist) = host.access_policy();
        assert!(
            !whitelist.admits(&[7; 32]) && !whitelist.admits(&[9; 32]),
            "{whitelist:?}"
        );
        assert!(whitelist.admits(&[1; 32]) && whitelist.admits(host.persona()));
        assert_eq!(blocklist, vec![[7; 32], [9; 32]]);

        // The pure list also drops a block that only arrives as an admission.
        let list = access_allowlist(&[42; 32], &[], &[[9; 32], [5; 32]], &[[9; 32]]);
        assert_eq!(list, vec![[42; 32], [5; 32]]);
    }

    #[test]
    fn a_blocked_persona_holding_the_bearer_gets_silence() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(&hub, vec![]);
            host.set_book(Vec::new(), vec![caller.persona.public_key().to_bytes()]);
            let bearer = hex::encode(host.invite().bearer);
            let joiner_relay = hub.client();
            let offer = caller
                .offer(Some(bearer), crate::protocol::PROTOCOL_VERSION)
                .await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();

            let events = host.poll(NOW, 1, 5);
            assert!(!events.is_empty(), "the offer was read");
            assert!(
                events.iter().all(|e| matches!(e, HostEvent::Dropped(_))),
                "a block beats a live bearer: {events:?}"
            );
            assert!(host.allowlist().is_empty());
        });
    }

    #[test]
    fn a_stranger_with_no_bearer_gets_no_answer_at_all() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(&hub, vec![]);
            let joiner_relay = hub.client();
            joiner_relay
                .subscribe(KIND_JOIN_ANSWER, &caller.runtime.public_key().to_hex())
                .unwrap();
            let offer = caller.offer(None, crate::protocol::PROTOCOL_VERSION).await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();

            let events = host.poll(NOW, 1, 5);
            assert!(
                events.iter().any(|e| matches!(e, HostEvent::Dropped(_))),
                "{events:?}"
            );
            assert!(
                joiner_relay.try_recv().is_none(),
                "SILENCE — no answer to a stranger"
            );
            assert!(host.allowlist().is_empty());
        });
    }

    #[test]
    fn a_ken_contact_is_refused_like_a_stranger() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(
                &hub,
                vec![a_contact(
                    caller.persona.public_key().to_bytes(),
                    crate::comms::Tier::Ken,
                )],
            );
            let joiner_relay = hub.client();
            joiner_relay
                .subscribe(KIND_JOIN_ANSWER, &caller.runtime.public_key().to_hex())
                .unwrap();
            let offer = caller.offer(None, crate::protocol::PROTOCOL_VERSION).await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();
            host.poll(NOW, 1, 5);
            assert!(
                joiner_relay.try_recv().is_none(),
                "ken is hear-only, not play-with"
            );
            assert!(host.allowlist().is_empty());
        });
    }

    #[test]
    fn a_protocol_mismatch_is_explained_rather_than_ignored() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(
                &hub,
                vec![a_contact(
                    caller.persona.public_key().to_bytes(),
                    crate::comms::Tier::Kin,
                )],
            );
            let joiner_relay = hub.client();
            joiner_relay
                .subscribe(KIND_JOIN_ANSWER, &caller.runtime.public_key().to_hex())
                .unwrap();
            let offer = caller
                .offer(None, crate::protocol::PROTOCOL_VERSION + 1)
                .await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();
            host.poll(NOW, 1, 5);

            let answer_ev = joiner_relay.try_recv().expect("a mismatch IS answered");
            let answer =
                crate::rendezvous::payload::open_answer(&caller.runtime, &answer_ev).unwrap();
            assert!(!answer.accepted);
            assert_eq!(answer.reason.as_deref(), Some("protocol-mismatch"));
            assert!(
                host.allowlist().is_empty(),
                "an explained refusal is still a refusal"
            );
        });
    }

    #[test]
    fn a_stranger_with_a_protocol_mismatch_still_gets_silence() {
        rt().block_on(async {
            // WHO is decided before WHETHER. A stranger must not learn that
            // anybody is hosting here — not even by being told their build is
            // old, an answer that would carry the host's persona, attestation
            // and world name.
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(&hub, vec![]);
            let joiner_relay = hub.client();
            joiner_relay
                .subscribe(KIND_JOIN_ANSWER, &caller.runtime.public_key().to_hex())
                .unwrap();
            let offer = caller
                .offer(None, crate::protocol::PROTOCOL_VERSION + 1)
                .await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();
            host.poll(NOW, 1, 5);

            assert!(
                joiner_relay.try_recv().is_none(),
                "SILENCE — a stranger's protocol is never the host's business to answer"
            );
            assert!(host.allowlist().is_empty());
        });
    }

    #[test]
    fn a_refused_invitee_is_explained_and_not_let_in() {
        rt().block_on(async {
            // The bearer is good but the build is old: explained, and they are
            // not on the list. The bearer is still free for the retry.
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(&hub, vec![]);
            let bearer = hex::encode(host.invite().bearer);
            let joiner_relay = hub.client();
            joiner_relay
                .subscribe(KIND_JOIN_ANSWER, &caller.runtime.public_key().to_hex())
                .unwrap();
            let offer = caller
                .offer(Some(bearer), crate::protocol::PROTOCOL_VERSION + 1)
                .await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();
            host.poll(NOW, 1, 5);

            let answer_ev = joiner_relay.try_recv().expect("an invitee IS answered");
            let answer =
                crate::rendezvous::payload::open_answer(&caller.runtime, &answer_ev).unwrap();
            assert_eq!(answer.reason.as_deref(), Some("protocol-mismatch"));
            assert!(host.allowlist().is_empty());
        });
    }

    #[test]
    fn a_flood_is_drained_across_polls_and_the_real_offer_still_lands() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(
                &hub,
                vec![a_contact(
                    caller.persona.public_key().to_bytes(),
                    crate::comms::Tier::Kith,
                )],
            );
            let joiner_relay = hub.client();
            for _ in 0..50 {
                let junk = nostr::EventBuilder::new(
                    nostr::Kind::Custom(KIND_JOIN_OFFER),
                    "not-a-ciphertext",
                )
                .tags([nostr::Tag::public_key(host_runtime.public_key())])
                .sign(&Keys::generate())
                .await
                .unwrap();
                joiner_relay.publish(&junk).unwrap();
            }
            let offer = caller.offer(None, crate::protocol::PROTOCOL_VERSION).await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();

            let first = host.poll(NOW, 1, 5);
            assert_eq!(
                first.len(),
                MAX_EVENTS_PER_POLL,
                "one frame must not swallow a flood: {first:?}"
            );
            assert!(
                !first
                    .iter()
                    .any(|e| matches!(e, HostEvent::Admitted { .. })),
                "the genuine offer is behind 50 junk ones"
            );

            let mut dropped = first.len();
            let mut admitted = false;
            for _ in 0..4 {
                for e in host.poll(NOW, 1, 5) {
                    match e {
                        HostEvent::Dropped(_) => dropped += 1,
                        HostEvent::Admitted { .. } => admitted = true,
                        _ => {}
                    }
                }
            }
            assert_eq!(
                dropped, 50,
                "every junk event is accounted for, across polls"
            );
            assert!(admitted, "and the real offer is not starved");
        });
    }

    #[test]
    fn a_forged_event_id_is_dropped_before_it_can_squat_the_dedupe_set() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(
                &hub,
                vec![a_contact(
                    caller.persona.public_key().to_bytes(),
                    crate::comms::Tier::Kith,
                )],
            );
            let joiner_relay = hub.client();
            let offer = caller.offer(None, crate::protocol::PROTOCOL_VERSION).await;
            let genuine = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();

            // Junk wearing the genuine offer's id, delivered first. If the id
            // were taken on trust it would be remembered, and the real offer
            // behind it silently dropped as a duplicate.
            let mut forged =
                nostr::EventBuilder::new(nostr::Kind::Custom(KIND_JOIN_OFFER), "not-a-ciphertext")
                    .tags([nostr::Tag::public_key(host_runtime.public_key())])
                    .sign(&Keys::generate())
                    .await
                    .unwrap();
            forged.id = genuine.id;
            joiner_relay.publish(&forged).unwrap();
            joiner_relay.publish(&genuine).unwrap();

            let events = host.poll(NOW, 1, 5);
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, HostEvent::Admitted { .. })),
                "the genuine offer must survive the squatter: {events:?}"
            );
        });
    }

    #[test]
    fn a_full_world_is_explained() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(
                &hub,
                vec![a_contact(
                    caller.persona.public_key().to_bytes(),
                    crate::comms::Tier::Kin,
                )],
            );
            let joiner_relay = hub.client();
            joiner_relay
                .subscribe(KIND_JOIN_ANSWER, &caller.runtime.public_key().to_hex())
                .unwrap();
            let offer = caller.offer(None, crate::protocol::PROTOCOL_VERSION).await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();
            host.poll(NOW, 5, 5); // players == capacity

            let answer_ev = joiner_relay.try_recv().expect("a full world IS answered");
            let answer =
                crate::rendezvous::payload::open_answer(&caller.runtime, &answer_ev).unwrap();
            assert_eq!(answer.reason.as_deref(), Some("full"));
        });
    }

    #[test]
    fn an_accepted_answer_carries_the_hosts_candidates_and_world_name() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(
                &hub,
                vec![a_contact(
                    caller.persona.public_key().to_bytes(),
                    crate::comms::Tier::Kith,
                )],
            );
            let joiner_relay = hub.client();
            joiner_relay
                .subscribe(KIND_JOIN_ANSWER, &caller.runtime.public_key().to_hex())
                .unwrap();
            let offer = caller.offer(None, crate::protocol::PROTOCOL_VERSION).await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();
            host.poll(NOW, 1, 5);

            let answer_ev = joiner_relay.try_recv().unwrap();
            let answer =
                crate::rendezvous::payload::open_answer(&caller.runtime, &answer_ev).unwrap();
            assert!(answer.accepted);
            assert_eq!(
                answer.session, offer.session,
                "the answer is tied to the offer"
            );
            assert_eq!(answer.world_name, "Ivy's Hollow");
            assert_eq!(answer.candidates[0].addr, "127.0.0.1:7700");
        });
    }

    #[test]
    fn the_same_offer_twice_is_admitted_once() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(
                &hub,
                vec![a_contact(
                    caller.persona.public_key().to_bytes(),
                    crate::comms::Tier::Kith,
                )],
            );
            let joiner_relay = hub.client();
            let offer = caller.offer(None, crate::protocol::PROTOCOL_VERSION).await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();
            joiner_relay.publish(&ev).unwrap();
            let events = host.poll(NOW, 1, 5);
            let admits = events
                .iter()
                .filter(|e| matches!(e, HostEvent::Admitted { .. }))
                .count();
            assert_eq!(admits, 1, "the replay guard holds: {events:?}");
        });
    }

    // ─── One link, one person, this world only ───

    /// Publish `offer` from `caller` to the host, and read the host's reply on
    /// a relay subscribed as the caller.
    async fn call_with(
        hub: &FakeRelayHub,
        caller: &Caller,
        host_runtime: &Keys,
        offer: &Offer,
    ) -> Box<dyn RendezvousRelay> {
        let relay = hub.client();
        relay
            .subscribe(KIND_JOIN_ANSWER, &caller.runtime.public_key().to_hex())
            .unwrap();
        let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), offer)
            .await
            .unwrap();
        relay.publish(&ev).unwrap();
        Box::new(relay)
    }

    /// `caller` presents `bearer` at time `at`: what the host did, and any reply.
    async fn present(
        host: &mut OnlineHost,
        hub: &FakeRelayHub,
        caller: &Caller,
        host_runtime: &Keys,
        bearer: Option<[u8; 16]>,
        at: u64,
    ) -> (Vec<HostEvent>, Option<nostr::Event>) {
        let mut offer = caller
            .offer(bearer.map(hex::encode), crate::protocol::PROTOCOL_VERSION)
            .await;
        offer.sent_at = at;
        let relay = call_with(hub, caller, host_runtime, &offer).await;
        let events = host.poll(at, 1, 5);
        (events, relay.try_recv())
    }

    fn admitted_by_invite(events: &[HostEvent]) -> bool {
        events.iter().any(|e| {
            matches!(e, HostEvent::Admitted { reason: AdmitReason::ByInvite, .. })
        })
    }

    #[test]
    fn a_forwarded_link_gets_a_second_persona_the_silence_of_no_invite() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let (first, second) = (Caller::new(), Caller::new());
            let (mut host, host_runtime) = a_host(&hub, vec![]);
            let bearer = host.invite().bearer;

            let (events, answer) =
                present(&mut host, &hub, &first, &host_runtime, Some(bearer), NOW).await;
            assert!(admitted_by_invite(&events), "{events:?}");
            assert!(answer.is_some(), "the first persona is answered");

            let (events, answer) =
                present(&mut host, &hub, &second, &host_runtime, Some(bearer), NOW).await;
            assert!(answer.is_none(), "no reply of any kind to a second persona");
            assert!(
                events.iter().all(|e| matches!(e, HostEvent::Dropped(_))),
                "{events:?}"
            );
            let second_pk = second.persona.public_key().to_bytes();
            assert!(!host.allowlist().contains(&second_pk));
            assert!(!host.access_policy().0.admits(&second_pk));
        });
    }

    #[test]
    fn the_first_persona_can_reconnect_with_its_link_until_it_expires() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let first = Caller::new();
            let (mut host, host_runtime) = a_host(&hub, vec![]);
            let (bearer, expires_at) = (host.invite().bearer, host.invite().expires_at);
            let first_pk = first.persona.public_key().to_bytes();

            let (events, _) =
                present(&mut host, &hub, &first, &host_runtime, Some(bearer), NOW).await;
            assert!(admitted_by_invite(&events), "{events:?}");

            // A dropped connection: back with the same link, a new session.
            let (events, answer) =
                present(&mut host, &hub, &first, &host_runtime, Some(bearer), NOW + 60).await;
            assert!(admitted_by_invite(&events), "by the link, as before: {events:?}");
            assert!(answer.is_some(), "the owner is answered");

            // The last good second, then silence, for the owner too.
            let (events, _) =
                present(&mut host, &hub, &first, &host_runtime, Some(bearer), expires_at).await;
            assert!(admitted_by_invite(&events), "{events:?}");
            let (events, answer) =
                present(&mut host, &hub, &first, &host_runtime, Some(bearer), expires_at + 1).await;
            assert!(!events.iter().any(|e| matches!(e, HostEvent::Admitted { .. })), "{events:?}");
            assert!(answer.is_none(), "an expired link is silence");

            // The session entry is what still names them on the server.
            assert!(host.access_policy().0.admits(&first_pk));
        });
    }

    #[test]
    fn a_guest_is_written_nowhere_and_is_not_kith() {
        rt().block_on(async {
            use crate::comms::{CommsLevel, Tier, hear_ok, speak_ok};
            let hub = FakeRelayHub::new();
            let guest = Caller::new();
            let (mut host, host_runtime) = a_host(&hub, vec![]);
            let bearer = host.invite().bearer;
            let guest_pk = guest.persona.public_key().to_bytes();

            let (events, _) =
                present(&mut host, &hub, &guest, &host_runtime, Some(bearer), NOW).await;
            assert!(admitted_by_invite(&events), "{events:?}");

            assert!(crate::contacts::find(host.book(), &guest_pk).is_none(), "no row in the book");
            assert!(
                crate::contacts::find(&crate::contacts::load_mirror(&crate::contacts::mirror_path()), &guest_pk).is_none(),
                "no row in the mirror"
            );
            assert!(host.access_policy().0.admits(&guest_pk), "but they are let in");

            // At the safe default a line reaches nobody in either direction.
            let tier = crate::contacts::tier_of(host.book(), &guest_pk);
            assert_eq!(tier, Tier::Stranger);
            assert!(!speak_ok(CommsLevel::Approved, tier));
            assert!(!hear_ok(CommsLevel::Approved, tier));
        });
    }

    #[test]
    fn being_let_in_grants_nothing_in_another_world_or_session() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let guest = Caller::new();
            let (mut first_world, first_runtime) = a_host(&hub, vec![]);
            let bearer = first_world.invite().bearer;
            let (events, _) =
                present(&mut first_world, &hub, &guest, &first_runtime, Some(bearer), NOW).await;
            assert!(admitted_by_invite(&events), "{events:?}");

            // The next world starts from the book this one leaves behind.
            let (mut next_world, next_runtime) = a_host(&hub, first_world.book().to_vec());
            let guest_pk = guest.persona.public_key().to_bytes();
            let (events, answer) =
                present(&mut next_world, &hub, &guest, &next_runtime, None, NOW).await;
            assert!(!events.iter().any(|e| matches!(e, HostEvent::Admitted { .. })), "{events:?}");
            assert!(answer.is_none());
            let (events, answer) =
                present(&mut next_world, &hub, &guest, &next_runtime, Some(bearer), NOW).await;
            assert!(!events.iter().any(|e| matches!(e, HostEvent::Admitted { .. })), "{events:?}");
            assert!(answer.is_none());
            assert!(!next_world.access_policy().0.admits(&guest_pk));
        });
    }

    #[test]
    fn a_refreshed_book_keeps_this_session_s_guest_and_a_block_removes_them() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let (guest, other) = (Caller::new(), Caller::new());
            let (mut host, host_runtime) = a_host(&hub, vec![]);
            let bearer = host.invite().bearer;
            let guest_pk = guest.persona.public_key().to_bytes();
            let (events, _) =
                present(&mut host, &hub, &guest, &host_runtime, Some(bearer), NOW).await;
            assert!(admitted_by_invite(&events), "{events:?}");

            host.set_book(vec![], vec![]);
            assert!(host.allowlist().contains(&guest_pk), "a book without them does not remove them");
            assert!(host.access_policy().0.admits(&guest_pk));
            assert!(crate::contacts::find(host.book(), &guest_pk).is_none());

            host.set_book(vec![], vec![guest_pk]);
            assert!(!host.allowlist().contains(&guest_pk), "a block does");
            assert!(!host.access_policy().0.admits(&guest_pk));

            // A block does not free the link for somebody else.
            let (events, answer) =
                present(&mut host, &hub, &other, &host_runtime, Some(bearer), NOW).await;
            assert!(!events.iter().any(|e| matches!(e, HostEvent::Admitted { .. })), "{events:?}");
            assert!(answer.is_none());
        });
    }

    #[test]
    fn a_ken_contact_let_in_by_a_link_stays_ken() {
        rt().block_on(async {
            use crate::comms::Tier;
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let pk = caller.persona.public_key().to_bytes();
            let (mut host, host_runtime) = a_host(&hub, vec![a_contact(pk, Tier::Ken)]);
            let bearer = host.invite().bearer;
            let (events, _) =
                present(&mut host, &hub, &caller, &host_runtime, Some(bearer), NOW).await;
            assert!(admitted_by_invite(&events), "{events:?}");
            let row = crate::contacts::find(host.book(), &pk).expect("their row is untouched");
            assert_eq!(row.tier, Tier::Ken);
            assert_eq!(row.added_via, AddedVia::Kenspeckle);
            assert_eq!(row.display_name.as_deref(), Some("Friend"));
        });
    }

    /// A real link join carries the handover context of the link it came from.
    /// The guest of an earlier link must get back in after the host made a new
    /// one for somebody else, and be answered in that link's own context.
    #[test]
    fn a_fresh_link_for_somebody_else_does_not_lock_the_first_guest_out() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let (first, next) = (Caller::new(), Caller::new());
            let (mut host, host_runtime) = a_host(&hub, vec![]);
            let (old_bearer, old_context) =
                (host.invite().bearer, host.handover_context(NOW).unwrap());
            let mut offer = first
                .offer(Some(hex::encode(old_bearer)), crate::protocol::PROTOCOL_VERSION)
                .await;
            offer.handover = Some(old_context);
            let _relay = call_with(&hub, &first, &host_runtime, &offer).await;
            assert!(admitted_by_invite(&host.poll(NOW, 1, 5)));

            let new_bearer = host.mint_fresh_invite(NOW + 60).unwrap().bearer;

            // The next friend comes in on the new link.
            let (events, _) =
                present(&mut host, &hub, &next, &host_runtime, Some(new_bearer), NOW + 60).await;
            assert!(admitted_by_invite(&events), "{events:?}");

            // The first guest reconnects on the old one.
            let mut offer = first
                .offer(Some(hex::encode(old_bearer)), crate::protocol::PROTOCOL_VERSION)
                .await;
            offer.sent_at = NOW + 60;
            offer.handover = Some(old_context);
            let relay = call_with(&hub, &first, &host_runtime, &offer).await;
            let events = host.poll(NOW + 60, 1, 5);
            assert!(admitted_by_invite(&events), "{events:?}");
            let answer = relay.try_recv().expect("the owner is answered");
            let answer =
                crate::rendezvous::payload::open_answer(&first.runtime, &answer).unwrap();
            assert!(answer.accepted);
            assert_eq!(answer.handover, Some(old_context), "in the context of their own link");

            // Nobody else gets the old link, with its context or without.
            let stranger = Caller::new();
            let mut offer = stranger
                .offer(Some(hex::encode(old_bearer)), crate::protocol::PROTOCOL_VERSION)
                .await;
            offer.sent_at = NOW + 60;
            offer.handover = Some(old_context);
            let relay = call_with(&hub, &stranger, &host_runtime, &offer).await;
            let events = host.poll(NOW + 60, 1, 5);
            assert!(!events.iter().any(|e| matches!(e, HostEvent::Admitted { .. })), "{events:?}");
            assert!(relay.try_recv().is_none());
        });
    }

    /// A rotation means the bearer may have left this machine: no grace, for
    /// its owner either. They stay on this session's list if they are seated.
    #[test]
    fn rotating_the_bearer_ends_the_link_for_its_owner_too() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let guest = Caller::new();
            let (mut host, host_runtime) = a_host(&hub, vec![]);
            let old = host.invite().bearer;
            let guest_pk = guest.persona.public_key().to_bytes();
            let (events, _) = present(&mut host, &hub, &guest, &host_runtime, Some(old), NOW).await;
            assert!(admitted_by_invite(&events), "{events:?}");

            host.rotate_bearer();

            let (events, answer) =
                present(&mut host, &hub, &guest, &host_runtime, Some(old), NOW + 1).await;
            assert!(!events.iter().any(|e| matches!(e, HostEvent::Admitted { .. })), "{events:?}");
            assert!(answer.is_none());
            assert!(host.access_policy().0.admits(&guest_pk), "a seated guest isn't thrown out");
        });
    }

    #[test]
    fn a_refused_invitee_does_not_use_the_link_up() {
        rt().block_on(async {
            // A full world or an old build is explained, and the link stays
            // free for the retry: it would be useless to the real invitee if a
            // refused call had already spent it.
            let hub = FakeRelayHub::new();
            let (first, second) = (Caller::new(), Caller::new());
            let (mut host, host_runtime) = a_host(&hub, vec![]);
            let bearer = host.invite().bearer;
            let offer = first
                .offer(Some(hex::encode(bearer)), crate::protocol::PROTOCOL_VERSION + 1)
                .await;
            let _relay = call_with(&hub, &first, &host_runtime, &offer).await;
            assert!(!admitted_by_invite(&host.poll(NOW, 1, 5)));

            let (events, _) =
                present(&mut host, &hub, &second, &host_runtime, Some(bearer), NOW).await;
            assert!(admitted_by_invite(&events), "{events:?}");
        });
    }

    #[test]
    fn the_unreachable_warning_is_the_approved_copy() {
        assert_eq!(
            unreachable_warning(),
            "Friends outside your home probably can't reach you. Turn on UPnP on your router."
        );
    }
}
