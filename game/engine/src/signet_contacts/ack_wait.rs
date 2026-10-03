//! Waiting for the pairing ack — WIRE.md §3 "Ack delivery", ported from
//! upstream `awaitPairingAck` (`src/client.ts`):
//!
//! - two SEPARATE filters, never merged: kind 21237 `#p=[app pubkey]` (the
//!   ephemeral ack) and kind 30078 `#d=[ack_tag(challenge)]` (its stored copy)
//!   — a merged `#p` query would also pull this app's projections;
//! - up to `ACK_CANDIDATE_LIMIT` (10) candidates per filter, newest first;
//! - per candidate: carrier freshness → NIP-44 decrypt → challenge gate
//!   (`decrypt_ack_event`); the ack author is never pinned;
//! - at most `ACK_ATTEMPT_CAP` (32) candidates are ever tried;
//! - the stored filter pages backward (`until = oldest − 1`, ≤ 3 pages) when a
//!   full page is all already-tried ids;
//! - each iteration polls FIRST, then checks the deadline / cancel / cap.
//!
//! The relay is behind [`AckRelay`] so every rule above is unit-tested with a
//! fake; the real socket lives in `relay.rs`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nostr::{Event, EventId, Keys};
use serde_json::{json, Value};

use crate::signet::contacts_wire::constants::{
    ACK_ATTEMPT_CAP, ACK_CANDIDATE_LIMIT, ACK_KIND, ACK_STORED_KIND,
};
use crate::signet::contacts_wire::{ack_event_is_fresh, ack_tag, decrypt_ack_event, Ack, Capability};

/// Bounded backward paging per poll (upstream `MAX_ACK_PAGES`).
pub const MAX_ACK_PAGES: usize = 3;
/// How long one poll pass listens live before the next pass (upstream
/// `pollMs` default).
pub const POLL_LISTEN: Duration = Duration::from_secs(2);

/// The relay half of the waiter.
pub trait AckRelay {
    /// Query both filters (as two separate subscriptions), then keep both open
    /// for up to `listen`, returning early when anything arrives live.
    /// Returns `(ephemeral + anything heard live, the stored filter's page)`.
    /// A failure is an empty answer, never an error.
    fn poll(&mut self, ephemeral: &Value, stored: &Value, listen: Duration) -> (Vec<Event>, Vec<Event>);
    /// One more page of the stored filter (paging).
    fn fetch(&mut self, filter: &Value) -> Vec<Event>;
}

pub fn ephemeral_filter(app_pubkey_hex: &str) -> Value {
    json!({ "kinds": [ACK_KIND], "#p": [app_pubkey_hex], "limit": ACK_CANDIDATE_LIMIT })
}

pub fn stored_filter(challenge: &str, until: Option<u64>) -> Value {
    let mut f = json!({ "kinds": [ACK_STORED_KIND], "#d": [ack_tag(challenge)], "limit": ACK_CANDIDATE_LIMIT });
    if let Some(u) = until {
        f["until"] = json!(u);
    }
    f
}

/// Newest first, one per id, at most `ACK_CANDIDATE_LIMIT`.
pub fn newest_first(mut events: Vec<Event>) -> Vec<Event> {
    events.sort_by_key(|e| std::cmp::Reverse(e.created_at));
    let mut seen = HashSet::new();
    events.retain(|e| seen.insert(e.id));
    events.truncate(ACK_CANDIDATE_LIMIT);
    events
}

/// The paging trigger: a FULL page whose every id was already tried means a
/// flood may be hiding the genuine (older) stored ack. Returns the `until` for
/// the next page — one second before the oldest event seen, because a relay's
/// `until` is inclusive and the same value would return the same page forever.
pub fn page_back_until(page: &[Event], attempted: &HashSet<EventId>) -> Option<u64> {
    if page.len() < ACK_CANDIDATE_LIMIT || !page.iter().all(|e| attempted.contains(&e.id)) {
        return None;
    }
    let oldest = page.iter().map(|e| e.created_at.as_secs()).min()?;
    oldest.checked_sub(1)
}

/// Fetch the stored filter's page, paging backward while [`page_back_until`]
/// says so, at most `MAX_ACK_PAGES` pages in all (the first included).
pub fn page_stored(
    relay: &mut dyn AckRelay,
    first: Vec<Event>,
    challenge: &str,
    attempted: &HashSet<EventId>,
) -> Vec<Event> {
    let mut page = newest_first(first);
    let mut fetched = 1;
    while fetched < MAX_ACK_PAGES {
        let Some(until) = page_back_until(&page, attempted) else { break };
        page = newest_first(relay.fetch(&stored_filter(challenge, Some(until))));
        fetched += 1;
    }
    page
}

/// What one batch of candidates produced.
#[derive(Debug, PartialEq, Eq)]
pub enum CandidateOutcome {
    Found(Ack),
    /// `ACK_ATTEMPT_CAP` candidates have been tried; the pairing is over.
    CapHit,
    Nothing,
}

/// Try `candidates` in order. A candidate is skipped (and costs nothing) when
/// stale or already tried; otherwise it is recorded in `attempted` and opened.
pub fn process_candidates(
    candidates: &[Event],
    attempted: &mut HashSet<EventId>,
    now: u64,
    app_keys: &Keys,
    challenge: &str,
    requested: &[Capability],
) -> CandidateOutcome {
    for ev in candidates {
        if attempted.len() >= ACK_ATTEMPT_CAP {
            return CandidateOutcome::CapHit;
        }
        if !ack_event_is_fresh(ev.created_at.as_secs(), now) || attempted.contains(&ev.id) {
            continue;
        }
        attempted.insert(ev.id);
        if let Some(ack) = decrypt_ack_event(ev, app_keys, challenge, requested) {
            return CandidateOutcome::Found(ack);
        }
    }
    CandidateOutcome::Nothing
}

/// How a wait ended.
#[derive(Debug, PartialEq, Eq)]
pub enum AckWait {
    Found(Ack),
    TimedOut,
    Cancelled,
}

/// The waiter loop. `now` is the unix-seconds clock; `deadline` is unix
/// seconds. Polls first, then checks deadline / cancel / cap, so a process
/// that slept past the deadline still gets one last look.
pub fn await_ack(
    relay: &mut dyn AckRelay,
    app_keys: &Keys,
    challenge: &str,
    requested: &[Capability],
    deadline: u64,
    now: &dyn Fn() -> u64,
    cancel: &AtomicBool,
) -> AckWait {
    let eph = ephemeral_filter(&app_keys.public_key().to_hex());
    let stored = stored_filter(challenge, None);
    let mut attempted: HashSet<EventId> = HashSet::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return AckWait::Cancelled;
        }
        let (live, first_stored) = relay.poll(&eph, &stored, POLL_LISTEN);
        let stored_page = page_stored(relay, first_stored, challenge, &attempted);
        let mut candidates = newest_first(live);
        candidates.extend(stored_page);
        match process_candidates(&candidates, &mut attempted, now(), app_keys, challenge, requested) {
            CandidateOutcome::Found(ack) => {
                return if cancel.load(Ordering::Relaxed) { AckWait::Cancelled } else { AckWait::Found(ack) };
            }
            CandidateOutcome::CapHit => return AckWait::TimedOut,
            CandidateOutcome::Nothing => {}
        }
        if cancel.load(Ordering::Relaxed) {
            return AckWait::Cancelled;
        }
        if now() >= deadline || attempted.len() >= ACK_ATTEMPT_CAP {
            return AckWait::TimedOut;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Kind, Tag, Timestamp};
    use std::cell::Cell;

    const CHALLENGE: &str = "0123456789abcdef0123456789abcdef";
    const REQ: [Capability; 3] = [Capability::ReadDirectory, Capability::ReadTier, Capability::BlocksRead];

    fn ack_json(challenge: &str) -> String {
        json!({
            "v": 2, "grantId": "1".repeat(32), "railPubkey": Keys::generate().public_key().to_hex(),
            "projectionTag": "2".repeat(32), "proposalTag": "3".repeat(32),
            "relay": "wss://relay.example.com", "grantedCapabilities": ["signet.contacts.read:directory"],
            "maxStalenessSeconds": 21600, "challenge": challenge,
        })
        .to_string()
    }

    fn sealed(kind: u16, plaintext: &str, to: &Keys, at: u64) -> Event {
        let eph = Keys::generate();
        let ct = nostr::nips::nip44::encrypt(eph.secret_key(), &to.public_key(), plaintext, nostr::nips::nip44::Version::V2).unwrap();
        EventBuilder::new(Kind::Custom(kind), ct)
            .tags([Tag::public_key(to.public_key())])
            .custom_created_at(Timestamp::from_secs(at))
            .sign_with_keys(&eph)
            .unwrap()
    }

    fn junk(at: u64) -> Event {
        EventBuilder::new(Kind::Custom(ACK_STORED_KIND), "junk")
            .custom_created_at(Timestamp::from_secs(at))
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn junk_and_a_forged_wrong_challenge_are_skipped_and_the_real_ack_wins() {
        let app = Keys::generate();
        let now = 10_000;
        let real = sealed(ACK_STORED_KIND, &ack_json(CHALLENGE), &app, now - 30);
        let forged = sealed(ACK_KIND, &ack_json(&"f".repeat(32)), &app, now - 1);
        // Clock-skewed into the future: newest of all, and still refused.
        let stale_real = sealed(ACK_KIND, &ack_json(CHALLENGE), &app, now + 301);
        let cands = newest_first(vec![junk(now - 2), real.clone(), forged, junk(now - 5), stale_real.clone()]);
        assert_eq!(cands.first().map(|e| e.id), Some(stale_real.id), "newest first");
        let mut attempted = HashSet::new();
        let out = process_candidates(&cands, &mut attempted, now, &app, CHALLENGE, &REQ);
        let CandidateOutcome::Found(ack) = out else { panic!("expected the real ack, got {out:?}") };
        assert_eq!(ack.challenge, CHALLENGE);
        assert!(!attempted.contains(&stale_real.id), "a stale carrier costs no attempt");
        // Every junk/forged event before the real one cost exactly one attempt.
        assert_eq!(attempted.len(), 4);
    }

    #[test]
    fn an_ack_addressed_to_someone_else_never_opens() {
        let app = Keys::generate();
        let other = Keys::generate();
        let ev = sealed(ACK_KIND, &ack_json(CHALLENGE), &other, 100);
        let mut attempted = HashSet::new();
        assert_eq!(process_candidates(&[ev], &mut attempted, 100, &app, CHALLENGE, &REQ), CandidateOutcome::Nothing);
    }

    #[test]
    fn the_attempt_cap_ends_the_pairing() {
        let app = Keys::generate();
        let evs: Vec<Event> = (0..40).map(|i| junk(1000 + i)).collect();
        let mut attempted = HashSet::new();
        assert_eq!(process_candidates(&evs, &mut attempted, 1000, &app, CHALLENGE, &REQ), CandidateOutcome::CapHit);
        assert_eq!(attempted.len(), ACK_ATTEMPT_CAP);
    }

    #[test]
    fn paging_triggers_only_on_a_full_page_of_already_tried_ids() {
        let page: Vec<Event> = (0..10).map(|i| junk(500 + i)).collect();
        let mut attempted: HashSet<EventId> = page.iter().map(|e| e.id).collect();
        assert_eq!(page_back_until(&page, &attempted), Some(499), "until = oldest - 1");
        assert_eq!(page_back_until(&page[..9], &attempted), None, "short page: nothing further back");
        attempted.remove(&page[3].id);
        assert_eq!(page_back_until(&page, &attempted), None, "an untried id is worth decrypting first");
    }

    struct Fake {
        polls: Vec<(Vec<Event>, Vec<Event>)>,
        pages: Vec<Vec<Event>>,
        fetched_untils: Vec<u64>,
        clock: std::rc::Rc<Cell<u64>>,
    }
    impl AckRelay for Fake {
        fn poll(&mut self, e: &Value, s: &Value, _l: Duration) -> (Vec<Event>, Vec<Event>) {
            // Two separate filters, never merged.
            assert_eq!(e["kinds"], json!([ACK_KIND]));
            assert!(e.get("#d").is_none());
            assert_eq!(s["kinds"], json!([ACK_STORED_KIND]));
            assert!(s.get("#p").is_none());
            self.clock.set(self.clock.get() + 2);
            if self.polls.is_empty() { (vec![], vec![]) } else { self.polls.remove(0) }
        }
        fn fetch(&mut self, f: &Value) -> Vec<Event> {
            self.fetched_untils.push(f["until"].as_u64().unwrap());
            if self.pages.is_empty() { vec![] } else { self.pages.remove(0) }
        }
    }

    #[test]
    fn a_flood_of_newer_stored_junk_is_paged_past_to_the_genuine_ack() {
        let app = Keys::generate();
        let clock = std::rc::Rc::new(Cell::new(10_000u64));
        let flood: Vec<Event> = (0..10).map(|i| junk(9_990 + i)).collect();
        let real = sealed(ACK_STORED_KIND, &ack_json(CHALLENGE), &app, 9_900);
        let mut fake = Fake {
            // Poll 1 sees the flood (tries them all); poll 2 sees the same flood
            // again, which triggers a page back that finds the real ack.
            polls: vec![(vec![], flood.clone()), (vec![], flood)],
            pages: vec![vec![real]],
            fetched_untils: vec![],
            clock: clock.clone(),
        };
        let c = clock.clone();
        let out = await_ack(&mut fake, &app, CHALLENGE, &REQ, 20_000, &move || c.get(), &AtomicBool::new(false));
        assert!(matches!(out, AckWait::Found(_)), "{out:?}");
        assert_eq!(fake.fetched_untils, vec![9_989]);
    }

    #[test]
    fn it_polls_once_more_even_when_already_past_the_deadline() {
        let app = Keys::generate();
        let clock = std::rc::Rc::new(Cell::new(10_000u64));
        let real = sealed(ACK_STORED_KIND, &ack_json(CHALLENGE), &app, 10_000);
        let mut fake = Fake { polls: vec![(vec![], vec![real])], pages: vec![], fetched_untils: vec![], clock: clock.clone() };
        let c = clock.clone();
        // Deadline already behind the clock: the poll still runs first.
        let out = await_ack(&mut fake, &app, CHALLENGE, &REQ, 9_000, &move || c.get(), &AtomicBool::new(false));
        assert!(matches!(out, AckWait::Found(_)));
    }

    #[test]
    fn it_times_out_and_honours_cancel() {
        let app = Keys::generate();
        let clock = std::rc::Rc::new(Cell::new(10_000u64));
        let mut fake = Fake { polls: vec![], pages: vec![], fetched_untils: vec![], clock: clock.clone() };
        let c = clock.clone();
        let out = await_ack(&mut fake, &app, CHALLENGE, &REQ, 10_010, &move || c.get(), &AtomicBool::new(false));
        assert_eq!(out, AckWait::TimedOut);
        let c = clock.clone();
        let out = await_ack(&mut fake, &app, CHALLENGE, &REQ, 99_999, &move || c.get(), &AtomicBool::new(true));
        assert_eq!(out, AckWait::Cancelled);
    }
}
