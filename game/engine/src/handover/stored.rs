//! Separate bounded one-shot stored operations; never replaces live rendezvous.
use super::{
    Error, Hash, Result,
    context::RootContext,
    crypto::{self, Verified, VerifiedPrepared},
};
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, JsonUtil, Kind, PublicKey};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

/// Share ONE instance across the entire operation, all relays, connect/send,
/// Prepared+Commit lookups and up to four hops. Invalid frames consume bytes.
pub struct Budget {
    deadline: tokio::time::Instant,
    bytes: usize,
    candidates: usize,
    seen: BTreeSet<Hash>,
    hops: u8,
    relays: BTreeMap<String, usize>,
    root: Option<Hash>,
}
impl Default for Budget {
    fn default() -> Self {
        Self::new()
    }
}
impl Budget {
    pub fn new() -> Self {
        Self {
            deadline: tokio::time::Instant::now() + Duration::from_secs(25),
            bytes: 0,
            candidates: 0,
            seen: BTreeSet::new(),
            hops: 0,
            relays: BTreeMap::new(),
            root: None,
        }
    }
    pub fn deadline(&self) -> tokio::time::Instant {
        self.deadline
    }
    /// CP9 review MEDIUM-1: one relay's own share of an attempt: the
    /// deadline becomes `share` from now (the resumer gives each relay of the
    /// root `relay_share(n)`, so one stalled relay can't eat the attempt).
    pub fn share_deadline(&mut self, share: Duration) {
        self.deadline = tokio::time::Instant::now() + share;
    }
    pub fn hop(&mut self) -> Result<()> {
        self.check()?;
        if self.hops >= 4 {
            return Err(Error::Limit("lookup hops"));
        }
        self.hops += 1;
        Ok(())
    }
    pub fn check(&self) -> Result<()> {
        if tokio::time::Instant::now() >= self.deadline {
            return Err(Error::Deadline);
        }
        Ok(())
    }
    pub fn charge_frame(&mut self, n: usize) -> Result<()> {
        self.check()?;
        self.bytes = self
            .bytes
            .checked_add(n)
            .ok_or(Error::Limit("lookup bytes"))?;
        if self.bytes > 4 * 1024 * 1024 {
            return Err(Error::Limit("lookup bytes"));
        }
        if n > super::RELAY_FRAME_LIMIT {
            return Err(Error::Limit("relay frame"));
        }
        Ok(())
    }
    fn authorise_relay(&mut self, url: &str, root: &RootContext) -> Result<()> {
        self.check()?;
        if !root.relays().iter().any(|r| r == url) || self.root.is_some_and(|h| h != root.digest())
        {
            return Err(Error::Invalid("relay context"));
        }
        self.root = Some(root.digest());
        self.register_relay(url)
    }
    fn register_relay(&mut self, url: &str) -> Result<()> {
        if !self.relays.contains_key(url) && self.relays.len() >= 8 {
            return Err(Error::Limit("lookup relays"));
        }
        self.relays.entry(url.to_owned()).or_default();
        Ok(())
    }
    fn candidate(&mut self, url: &str) -> Result<()> {
        self.check()?;
        self.candidates += 1;
        let local = self
            .relays
            .get_mut(url)
            .ok_or(Error::State("unregistered relay"))?;
        *local += 1;
        if *local > 16 || self.candidates > 128 {
            return Err(Error::Limit("lookup candidates"));
        }
        Ok(())
    }
    pub fn bytes_received(&self) -> usize {
        self.bytes
    }
    pub fn candidates_received(&self) -> usize {
        self.candidates
    }
}

/// Witness that a relay authorised by the root context returned exactly
/// this event on a fresh subscription. Coordinator ruling (CP6) 3: the field
/// and the constructor are private to this file, and the only producers are
/// `publish_readback` and `readback` (both end in `readback_inner`). Pinned
/// by `handover_stored_proof_has_exactly_two_producers`.
#[derive(Clone, Copy, Debug)]
pub struct StoredProof {
    hash: Hash,
}
impl StoredProof {
    pub fn event_hash(&self) -> Hash {
        self.hash
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Route {
    pub author: Hash,
    pub recipient: Hash,
    pub until: u64,
}
impl Route {
    pub fn matches(&self, ev: &Event) -> bool {
        let tags: Vec<_> = ev.tags.iter().collect();
        ev.pubkey.to_bytes() == self.author
            && ev.kind == Kind::Custom(super::STORED_KIND)
            && ev.created_at.as_secs() <= self.until
            && tags.len() == 1
            && tags[0].as_slice() == ["p", hex::encode(self.recipient).as_str()]
    }
}
fn subscription() -> Result<String> {
    let mut id = [0; 16];
    getrandom::fill(&mut id).map_err(|_| Error::Io)?;
    Ok(format!("axe-handover-{}", hex::encode(id)))
}
fn req(id: &str, route: Route) -> String {
    json!(["REQ",id,{"kinds":[super::STORED_KIND],"authors":[hex::encode(route.author)],"#p":[hex::encode(route.recipient)],"limit":16,"until":route.until}]).to_string()
}
fn config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(super::RELAY_FRAME_LIMIT))
        .max_frame_size(Some(super::RELAY_FRAME_LIMIT))
}
fn cancelled(flag: &AtomicBool) -> Result<()> {
    if flag.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// Cancellation and the SAME absolute deadline cover connect/send/read, not
/// only the inner receive loop. Dropping this future closes an unfinished socket.
async fn bounded<F: std::future::Future>(
    future: F,
    deadline: tokio::time::Instant,
    cancel: &AtomicBool,
) -> Result<F::Output> {
    tokio::pin!(future);
    loop {
        cancelled(cancel)?;
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Deadline);
        }
        tokio::select! {
            output=&mut future=>return Ok(output),
            _=tokio::time::sleep_until(deadline)=>return Err(Error::Deadline),
            _=tokio::time::sleep(Duration::from_millis(50))=>{},
        }
    }
}

/// Parsed only after frame byte gates. Unknown/malformed/foreign frames are
/// ignored but still charged. Serde's recursion limit is retained.
enum Frame {
    Event(Box<Event>),
    InvalidCandidate,
    Eose,
    Ok { hash: String, accepted: bool },
    Other,
}
fn parse(text: &str, id: &str) -> Frame {
    if text.len() > super::RELAY_FRAME_LIMIT {
        return Frame::Other;
    }
    let Ok(serde_json::Value::Array(a)) = serde_json::from_str(text) else {
        return Frame::Other;
    };
    match a.first().and_then(|v| v.as_str()) {
        Some("EVENT") if a.len() == 3 && a[1].as_str() == Some(id) => {
            if a[2].to_string().len() > super::EVENT_LIMIT {
                return Frame::InvalidCandidate;
            }
            match serde_json::from_value(a[2].clone()) {
                Ok(ev) => Frame::Event(Box::new(ev)),
                Err(_) => Frame::InvalidCandidate,
            }
        }
        Some("EOSE") if a.len() == 2 && a[1].as_str() == Some(id) => Frame::Eose,
        Some("OK") if a.len() == 4 => match (a[1].as_str(), a[2].as_bool()) {
            (Some(h), Some(accepted)) => Frame::Ok {
                hash: h.into(),
                accepted,
            },
            _ => Frame::Other,
        },
        _ => Frame::Other,
    }
}
async fn close<S>(
    ws: &mut tokio_tungstenite::WebSocketStream<S>,
    id: &str,
    deadline: tokio::time::Instant,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let deadline = deadline.min(tokio::time::Instant::now() + Duration::from_millis(50));
    let _ = tokio::time::timeout_at(
        deadline,
        ws.send(Message::Text(json!(["CLOSE", id]).to_string().into())),
    )
    .await;
    let _ = tokio::time::timeout_at(deadline, ws.close(None)).await;
}

pub async fn lookup(
    url: &str,
    root: &RootContext,
    route: Route,
    budget: &mut Budget,
    cancel: &AtomicBool,
) -> Result<Vec<Event>> {
    super::context::validate_relay(url)?;
    budget.authorise_relay(url, root)?;
    lookup_inner(url, route, budget, cancel).await
}
async fn lookup_inner(
    url: &str,
    route: Route,
    budget: &mut Budget,
    cancel: &AtomicBool,
) -> Result<Vec<Event>> {
    budget.check()?;
    cancelled(cancel)?;
    budget.register_relay(url)?;
    PublicKey::from_slice(&route.author)
        .and_then(|key| {
            key.xonly()?;
            Ok(key)
        })
        .map_err(|_| Error::Invalid("lookup author"))?;
    PublicKey::from_slice(&route.recipient)
        .and_then(|key| {
            key.xonly()?;
            Ok(key)
        })
        .map_err(|_| Error::Invalid("lookup recipient"))?;
    let deadline = budget.deadline();
    let id = subscription()?;
    let (mut ws, _) = bounded(
        connect_async_with_config(url, Some(config()), false),
        deadline,
        cancel,
    )
    .await?
    .map_err(|_| Error::Io)?;
    let work = async {
        ws.send(Message::Text(req(&id, route).into()))
            .await
            .map_err(|_| Error::Io)?;
        let mut out = Vec::new();
        loop {
            cancelled(cancel)?;
            budget.check()?;
            // Poll cancellation without resetting the enclosing absolute deadline.
            let msg = tokio::select! {m=ws.next()=>m,_=tokio::time::sleep(Duration::from_millis(50))=>continue};
            let Some(Ok(msg)) = msg else {
                return Err(Error::Io);
            };
            budget.charge_frame(msg.len())?;
            let Message::Text(text) = msg else { continue };
            match parse(&text, &id) {
                Frame::Event(ev) => {
                    budget.candidate(url)?;
                    if !route.matches(&ev) || ev.verify().is_err() {
                        continue;
                    }
                    if budget.seen.insert(ev.id.to_bytes()) {
                        out.push(*ev);
                    }
                }
                Frame::InvalidCandidate => {
                    budget.candidate(url)?;
                }
                Frame::Eose => return Ok(out),
                _ => {}
            }
        }
    };
    let result = bounded(work, deadline, cancel).await.and_then(|v| v);
    close(&mut ws, &id, deadline).await;
    result
}

/// Verify candidates, Prepared first, without trusting relay order. Maximum16
/// per relay/128 total bounds both verification passes. Prepared alone stays inert.
pub fn verify_candidates(
    events: &[Event],
    keys: &nostr::Keys,
    source: Hash,
    root: &RootContext,
    now: u64,
    budget: &Budget,
) -> Result<Vec<Verified>> {
    if events.len() > 128 {
        return Err(Error::Limit("verification candidates"));
    }
    let mut prepared: Vec<VerifiedPrepared> = Vec::new();
    let mut out = Vec::new();
    for ev in events {
        budget.check()?;
        if let Ok(Verified::Prepared(p)) = crypto::open(ev, keys, source, root, now, None) {
            prepared.push(p.clone());
            out.push(Verified::Prepared(p));
        }
    }
    for ev in events {
        budget.check()?;
        if let Ok(Verified::Committed(c)) =
            crypto::open_candidates(ev, keys, source, root, now, &prepared)
        {
            out.push(Verified::Committed(c));
        }
    }
    Ok(out)
}

/// Requires matching OK then a NEW stored subscription/readback of the exact
/// already crypto-verified event. Caller cannot turn queued publish into proof.
pub struct Publication<'a> {
    pub event: &'a Event,
    pub verified: &'a Verified,
    pub permit: Option<&'a super::phase::PublicationPermit>,
    pub route: Route,
}
pub async fn publish_readback(
    url: &str,
    root: &RootContext,
    publication: Publication<'_>,
    budget: &mut Budget,
    cancel: &AtomicBool,
) -> Result<StoredProof> {
    let Publication {
        event,
        verified,
        permit,
        route,
    } = publication;
    super::context::validate_relay(url)?;
    budget.authorise_relay(url, root)?;
    // CP6 review LOW-2: the same root binding as `readback`: a certificate
    // is only ever published on the relays of the root it belongs to.
    let core = match verified {
        Verified::Prepared(p) => p.core(),
        Verified::Committed(c) => c.core(),
    };
    if !same_root(core, root) {
        return Err(Error::Invalid("publication of another root's certificate"));
    }
    let hash = publication_allowed(event, verified, permit)?;
    if event.id.to_bytes() != hash
        || !route.matches(event)
        || event.verify().is_err()
        || event.as_json().len() > super::EVENT_LIMIT
    {
        return Err(Error::Invalid("publication proof"));
    }
    publish_inner(url, event, route, budget, cancel).await
}
/// Phase gate is checked even before opening a relay socket.
pub(crate) fn publication_allowed(
    event: &Event,
    verified: &Verified,
    permit: Option<&super::phase::PublicationPermit>,
) -> Result<Hash> {
    match verified {
        Verified::Prepared(p) => Ok(p.event_hash()),
        Verified::Committed(c) => {
            let p = permit.ok_or(Error::State("Committed publication before retirement"))?;
            if p.event_hash() != c.event_hash() || p.bytes() != event.as_json().as_bytes() {
                return Err(Error::State("publication retirement binding"));
            }
            Ok(c.event_hash())
        }
    }
}
/// CP9: the Committed publication of a retired source, from its durable
/// record alone (`phase::RetiredCommit`, no bearer): the same root binding,
/// route and exact fresh readback as `publish_readback`. The first relay
/// that witnesses it is the proof.
pub async fn publish_retired(
    url: &str,
    retired: &super::phase::RetiredCommit,
    budget: &mut Budget,
    cancel: &AtomicBool,
) -> Result<StoredProof> {
    let event = retired_publication(url, retired, budget)?;
    publish_inner(url, event, retired.route(), budget, cancel).await
}
/// CP9: after the proof, a best-effort copy of the same exact event to
/// another relay of the same root: publish and its matching OK, no
/// readback (no witness, never a proof).
pub async fn copy_retired(
    url: &str,
    retired: &super::phase::RetiredCommit,
    budget: &mut Budget,
    cancel: &AtomicBool,
) -> Result<()> {
    let event = retired_publication(url, retired, budget)?;
    publish_only(url, event, budget, cancel).await
}
fn retired_publication<'a>(
    url: &str,
    retired: &'a super::phase::RetiredCommit,
    budget: &mut Budget,
) -> Result<&'a Event> {
    super::context::validate_relay(url)?;
    budget.authorise_relay(url, retired.root())?;
    if !same_root(retired.core(), retired.root()) {
        return Err(Error::Invalid("publication of another root's certificate"));
    }
    let event = retired.event();
    if event.id.to_bytes() != retired.event_hash()
        || event.as_json().as_bytes() != retired.exact_bytes()
        || !retired.route().matches(event)
        || event.verify().is_err()
        || event.as_json().len() > super::EVENT_LIMIT
    {
        return Err(Error::Invalid("publication proof"));
    }
    Ok(event)
}
/// Publish and wait for the relay's matching OK (no readback).
async fn publish_only(
    url: &str,
    event: &Event,
    budget: &mut Budget,
    cancel: &AtomicBool,
) -> Result<()> {
    cancelled(cancel)?;
    budget.check()?;
    let deadline = budget.deadline();
    let (mut ws, _) = bounded(
        connect_async_with_config(url, Some(config()), false),
        deadline,
        cancel,
    )
    .await?
    .map_err(|_| Error::Io)?;
    let work = async {
        ws.send(Message::Text(json!(["EVENT", event]).to_string().into()))
            .await
            .map_err(|_| Error::Io)?;
        loop {
            budget.check()?;
            cancelled(cancel)?;
            let msg = tokio::select! {m=ws.next()=>m,_=tokio::time::sleep(Duration::from_millis(50))=>continue};
            let Some(Ok(msg)) = msg else {
                return Err(Error::Io);
            };
            budget.charge_frame(msg.len())?;
            let Message::Text(text) = msg else { continue };
            if let Frame::Ok { hash, accepted } = parse(&text, "")
                && hash == event.id.to_hex()
            {
                return if accepted {
                    Ok(())
                } else {
                    Err(Error::Invalid("relay rejected publication"))
                };
            }
        }
    };
    let result = bounded(work, deadline, cancel).await.and_then(|v| v);
    let _ = tokio::time::timeout_at(
        deadline.min(tokio::time::Instant::now() + Duration::from_millis(50)),
        ws.close(None),
    )
    .await;
    result
}
async fn publish_inner(
    url: &str,
    event: &Event,
    route: Route,
    budget: &mut Budget,
    cancel: &AtomicBool,
) -> Result<StoredProof> {
    publish_only(url, event, budget, cancel).await?;
    readback_inner(url, event, route, budget, cancel).await
}

/// The certificate belongs to this root context (context digest, root
/// runtime, persona and expiry).
fn same_root(core: &super::certificate::Core, root: &RootContext) -> bool {
    core.context == root.digest()
        && core.root_runtime == root.runtime()
        && core.root_persona == root.persona()
        && core.root_expiry == root.expiry()
}

/// Independent exact readback of an already crypto-verified Committed (no
/// publication): the recipient's `StoredProof` producer (coordinator ruling,
/// CP6), because it holds no publication permit. Constraints, each pinned by
/// a test: (1) only a `Verified::Committed` of the verified chain, by exact
/// event id/hash equality (the core asks the receiver for no Prepared
/// witness); (2) a NEW subscription on a relay the root context authorises,
/// charged to the caller's `Budget`, after the event is known, never a
/// cached/local copy or a publish connection's OK; (4) a different event
/// from the same author, or no relay EVENT at all, is never stored.
pub async fn readback(
    url: &str,
    root: &RootContext,
    event: &Event,
    verified: &Verified,
    route: Route,
    budget: &mut Budget,
    cancel: &AtomicBool,
) -> Result<StoredProof> {
    let Verified::Committed(c) = verified else {
        return Err(Error::State("readback witnesses only a verified Committed"));
    };
    super::context::validate_relay(url)?;
    budget.authorise_relay(url, root)?;
    // CP5 review MEDIUM-1: the verified Committed must belong to the root
    // context whose relays are being asked.
    if !same_root(c.core(), root) {
        return Err(Error::Invalid("readback of another root's certificate"));
    }
    if event.id.to_bytes() != c.event_hash()
        || !route.matches(event)
        || event.verify().is_err()
        || event.as_json().len() > super::EVENT_LIMIT
    {
        return Err(Error::Invalid("readback proof"));
    }
    readback_inner(url, event, route, budget, cancel).await
}
async fn readback_inner(
    url: &str,
    event: &Event,
    route: Route,
    budget: &mut Budget,
    cancel: &AtomicBool,
) -> Result<StoredProof> {
    // Dedupe must not suppress this independent exact readback because it was
    // seen in an earlier lookup. Restore seen set after this scoped query.
    let seen = std::mem::take(&mut budget.seen);
    let fetched = lookup_inner(url, route, budget, cancel).await;
    budget.seen.extend(seen);
    if fetched?
        .iter()
        .any(|ev| ev.id == event.id && ev.as_json() == event.as_json())
    {
        Ok(StoredProof {
            hash: event.id.to_bytes(),
        })
    } else {
        Err(Error::Invalid("missing exact stored readback"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Coordinator ruling (CP6) 3: `StoredProof`'s field and constructor are
    /// private to this file; the one struct literal is `readback_inner`'s, and
    /// only `publish_inner` (via `publish_readback`) and `readback` reach it.
    /// No other source file builds one.
    /// Every `StoredProof { hash ...` struct literal (any whitespace).
    fn literals(text: &str) -> Vec<usize> {
        text.match_indices("StoredProof")
            .filter(|(i, m)| {
                if text[..*i].ends_with("struct ") {
                    return false;
                }
                let rest = text[i + m.len()..].trim_start();
                rest.strip_prefix('{').is_some_and(|r| r.trim_start().starts_with("hash"))
            })
            .map(|(i, _)| i)
            .collect()
    }
    #[test]
    fn handover_stored_proof_has_exactly_two_producers() {
        let me = include_str!("stored.rs");
        let production = me.split("#[cfg(test)]").next().unwrap();
        assert!(production.contains("pub struct StoredProof {\n    hash: Hash,\n}"), "the field is not private");
        let imp = production.find("impl StoredProof {").unwrap();
        let imp = &production[imp..imp + production[imp..].find("\n}\n").unwrap()];
        assert!(!imp.contains("Self {") && !imp.contains("Self(") && !imp.contains("-> Self"), "StoredProof gained a constructor");
        let found = literals(production);
        assert_eq!(found.len(), 1, "a second StoredProof constructor");
        let inner = production.find("async fn readback_inner(").unwrap();
        assert!(found[0] > inner, "the constructor isn't readback_inner's");
        let callers: Vec<_> = production.match_indices("readback_inner(url").map(|(i, _)| i).collect();
        let publish = production.find("async fn publish_inner(").unwrap();
        let readback = production.find("pub async fn readback(").unwrap();
        assert_eq!(callers.len(), 2, "readback_inner has another caller");
        assert!(publish < readback && readback < inner);
        assert!(callers[0] > publish && callers[0] < readback, "publish_inner doesn't end in the readback");
        assert!(callers[1] > readback && callers[1] < inner, "readback doesn't use the one readback");
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, out)
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p)
                }
            }
        }
        let mut files = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        assert!(files.len() > 50);
        for f in files.iter().filter(|f| !f.ends_with("handover/stored.rs")) {
            let text = std::fs::read_to_string(f).unwrap();
            assert!(literals(&text).is_empty(), "{} builds a StoredProof", f.display());
        }
    }
    #[test]
    fn handover_stored_budget_counts_invalid_and_preserves_whole_operation() {
        let mut b = Budget::new();
        for _ in 0..4 {
            b.hop().unwrap();
        }
        assert!(b.hop().is_err());
        for _ in 0..128 {
            b.charge_frame(super::super::RELAY_FRAME_LIMIT).unwrap();
        }
        assert_eq!(b.bytes_received(), 4 * 1024 * 1024);
        assert!(b.charge_frame(1).is_err());
        let frame = req(
            "separate",
            Route {
                author: [1; 32],
                recipient: [2; 32],
                until: 900,
            },
        );
        assert!(!frame.contains("since"));
        assert!(frame.contains("separate"));
        assert!(matches!(parse("[\"EOSE\",\"other\"]", "own"), Frame::Other));
    }
    #[test]
    fn handover_stored_no_matching_ok_and_cancel_are_not_storage() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let key = nostr::Keys::generate();
            let ev = nostr::EventBuilder::new(Kind::Custom(super::super::STORED_KIND), "fixture")
                .tags([nostr::Tag::public_key(key.public_key())])
                .sign(&key)
                .await
                .unwrap();
            let route = Route {
                author: key.public_key().to_bytes(),
                recipient: key.public_key().to_bytes(),
                until: u64::MAX,
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (sock, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_async(sock).await.unwrap();
                let _ = ws.next().await;
                ws.send(Message::Text(
                    json!(["OK", "wrong event", true, ""]).to_string().into(),
                ))
                .await
                .unwrap();
                let _ = ws.close(None).await;
            });
            assert!(
                publish_inner(
                    &format!("ws://{address}"),
                    &ev,
                    route,
                    &mut Budget::new(),
                    &AtomicBool::new(false)
                )
                .await
                .is_err()
            );
            server.await.unwrap();
            assert!(matches!(
                lookup_inner(
                    "ws://127.0.0.1:1",
                    route,
                    &mut Budget::new(),
                    &AtomicBool::new(true)
                )
                .await,
                Err(Error::Cancelled)
            ));
            let mut expired = Budget::new();
            expired.deadline = tokio::time::Instant::now();
            assert!(matches!(
                lookup_inner(
                    "ws://127.0.0.1:1",
                    route,
                    &mut expired,
                    &AtomicBool::new(false)
                )
                .await,
                Err(Error::Deadline)
            ));
        });
    }
    #[test]
    fn handover_stored_loopback_requires_ack_and_independent_history() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let keys = nostr::Keys::generate();
            let recipient = nostr::Keys::generate();
            let ev = nostr::EventBuilder::new(
                Kind::Custom(super::super::STORED_KIND),
                "stored setup ciphertext",
            )
            .tags([nostr::Tag::public_key(recipient.public_key())])
            .sign(&keys)
            .await
            .unwrap();
            let route = Route {
                author: keys.public_key().to_bytes(),
                recipient: recipient.public_key().to_bytes(),
                until: u64::MAX,
            };
            for readback in [false, true] {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let expected = ev.clone();
                let server = tokio::spawn(async move {
                    // First connection: publish + matching OK. Second connection:
                    // historical query, independent of the now closed publisher.
                    let (sock, _) = listener.accept().await.unwrap();
                    let mut ws = tokio_tungstenite::accept_async(sock).await.unwrap();
                    let Message::Text(text) = ws.next().await.unwrap().unwrap() else {
                        panic!("publish")
                    };
                    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(v[0], "EVENT");
                    ws.send(Message::Text(
                        json!(["OK", expected.id.to_hex(), true, ""])
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                    let _ = ws.next().await;
                    let _ = ws.close(None).await;
                    let (sock, _) = listener.accept().await.unwrap();
                    let mut ws = tokio_tungstenite::accept_async(sock).await.unwrap();
                    let Message::Text(text) = ws.next().await.unwrap().unwrap() else {
                        panic!("query")
                    };
                    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
                    let id = v[1].as_str().unwrap();
                    assert_eq!(v[0], "REQ");
                    assert_eq!(v[2]["authors"][0], expected.pubkey.to_hex());
                    assert!(v[2].get("since").is_none());
                    if readback {
                        ws.send(Message::Text(
                            json!(["EVENT", id, expected]).to_string().into(),
                        ))
                        .await
                        .unwrap();
                    }
                    ws.send(Message::Text(json!(["EOSE", id]).to_string().into()))
                        .await
                        .unwrap();
                    let _ = ws.next().await;
                    let _ = ws.close(None).await;
                });
                let result = publish_inner(
                    &format!("ws://{address}"),
                    &ev,
                    route,
                    &mut Budget::new(),
                    &AtomicBool::new(false),
                )
                .await;
                assert_eq!(result.is_ok(), readback);
                server.await.unwrap();
            }
        });
    }
}
