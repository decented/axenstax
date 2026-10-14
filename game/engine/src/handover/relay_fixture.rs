//! Test-only in-process Nostr-subset relay on `ws://127.0.0.1:<port>`: the
//! one loopback fixture every stored-publication test uses (P1). It stores
//! signed events across connections (EVENT -> verify, store, OK), answers a
//! fresh subscription with matching stored events then EOSE (REQ with
//! kinds/authors/#p/until/limit), and ignores CLOSE. Never a real relay: it
//! listens on loopback only and lives as long as the value.
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, JsonUtil};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio_tungstenite::tungstenite::Message;

#[derive(Default)]
struct State {
    events: Vec<Event>,
    /// Answer every REQ with EOSE only (a relay that acknowledges but
    /// doesn't keep, or withholds what it keeps).
    withhold: bool,
    /// CP9 review MEDIUM-1: answer nothing at all (no OK, no EOSE).
    black_hole: bool,
    /// Publications received (EVENT frames), stored or not.
    received: usize,
    /// Subscriptions answered (REQ frames).
    queries: usize,
}

pub(crate) struct Relay {
    url: String,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn matches(ev: &Event, filter: &Value) -> bool {
    let has = |key: &str, value: Value| filter.get(key).and_then(Value::as_array).is_none_or(|a| a.contains(&value));
    let p_ok = filter.get("#p").and_then(Value::as_array).is_none_or(|want| {
        ev.tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "p" && want.contains(&Value::String(s[1].clone()))
        })
    });
    has("kinds", json!(ev.kind.as_u16()))
        && has("authors", json!(ev.pubkey.to_hex()))
        && p_ok
        && filter.get("until").and_then(Value::as_u64).is_none_or(|u| ev.created_at.as_secs() <= u)
}

async fn serve(sock: tokio::net::TcpStream, state: Arc<Mutex<State>>) {
    let Ok(mut ws) = tokio_tungstenite::accept_async(sock).await else { return };
    while let Some(Ok(msg)) = ws.next().await {
        let Message::Text(text) = msg else { continue };
        let Ok(Value::Array(a)) = serde_json::from_str::<Value>(&text) else { continue };
        if state.lock().unwrap().black_hole {
            continue;
        }
        let replies: Vec<Value> = match a.first().and_then(Value::as_str) {
            Some("EVENT") if a.len() == 2 => {
                let Ok(ev) = Event::from_json(a[1].to_string()) else { continue };
                let mut s = state.lock().unwrap();
                s.received += 1;
                let ok = ev.verify().is_ok();
                if ok && !s.events.iter().any(|e| e.id == ev.id) {
                    s.events.push(ev.clone());
                }
                vec![json!(["OK", ev.id.to_hex(), ok, ""])]
            }
            Some("REQ") if a.len() >= 3 => {
                let id = a[1].clone();
                let mut s = state.lock().unwrap();
                s.queries += 1;
                let limit = a[2].get("limit").and_then(Value::as_u64).unwrap_or(u64::MAX) as usize;
                let mut out: Vec<Value> = if s.withhold {
                    Vec::new()
                } else {
                    s.events
                        .iter()
                        .filter(|e| matches(e, &a[2]))
                        .take(limit)
                        .map(|e| json!(["EVENT", id, serde_json::from_str::<Value>(&e.as_json()).unwrap()]))
                        .collect()
                };
                out.push(json!(["EOSE", id]));
                out
            }
            _ => Vec::new(),
        };
        for r in replies {
            if ws.send(Message::Text(r.to_string().into())).await.is_err() {
                return;
            }
        }
    }
}

impl Relay {
    pub(crate) fn start() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback relay bind");
        listener.set_nonblocking(true).unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let state: Arc<Mutex<State>> = Default::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (st, sp) = (state.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("handover-relay-fixture".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                rt.block_on(async move {
                    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                    while !sp.load(Ordering::Relaxed) {
                        tokio::select! {
                            r = listener.accept() => if let Ok((sock, _)) = r {
                                tokio::spawn(serve(sock, st.clone()));
                            },
                            _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
                        }
                    }
                });
            })
            .unwrap();
        Self { url, state, stop, thread: Some(thread) }
    }
    pub(crate) fn url(&self) -> String {
        self.url.clone()
    }
    /// Store a signed event as if some client had published it (tests that
    /// need a relay to already hold a certificate).
    pub(crate) fn insert(&self, ev: Event) {
        let mut s = self.state.lock().unwrap();
        if !s.events.iter().any(|e| e.id == ev.id) {
            s.events.push(ev);
        }
    }
    pub(crate) fn black_hole(&self, on: bool) {
        self.state.lock().unwrap().black_hole = on;
    }
    pub(crate) fn withhold(&self, on: bool) {
        self.state.lock().unwrap().withhold = on;
    }
    pub(crate) fn stored(&self) -> Vec<Event> {
        self.state.lock().unwrap().events.clone()
    }
    pub(crate) fn received(&self) -> usize {
        self.state.lock().unwrap().received
    }
    pub(crate) fn queries(&self) -> usize {
        self.state.lock().unwrap().queries
    }
}
impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
