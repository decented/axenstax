//! WebSocket client transport (wasm32) — the browser PWA's pipe to the
//! dedicated server. The native half is `ws_transport.rs`.
//!
//! Backed by `web_sys::WebSocket` with binary `arraybuffer` frames. The browser
//! is single-threaded, so there is no background task: the `onmessage` callback
//! pushes inbound bytes into a shared queue that the game loop drains each frame
//! (`try_recv_from_server`), and outbound bytes sent before the socket opens are
//! buffered and flushed on `onopen`. The transport is legitimately `!Send`
//! (`Rc`/`WebSocket`), which the `MaybeSend` marker on the trait allows on wasm.

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{BinaryType, MessageEvent, WebSocket};

use crate::transport::{ClientTransport, Packet};

/// Browser WebSocket client. Holds the socket plus the shared inbound queue and
/// the closures (kept alive for the socket's lifetime).
pub struct WebSocketClientTransport {
    ws: WebSocket,
    inbound: Rc<RefCell<VecDeque<Packet>>>,
    open: Rc<RefCell<bool>>,
    /// Latched by `onclose` — the socket is gone for good.
    closed: Rc<RefCell<bool>>,
    outbox: Rc<RefCell<Vec<Vec<u8>>>>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_open: Closure<dyn FnMut()>,
    _on_error: Closure<dyn FnMut(web_sys::Event)>,
    _on_close: Closure<dyn FnMut(web_sys::Event)>,
}

impl ClientTransport for WebSocketClientTransport {
    fn send_to_server(&self, data: &[u8]) {
        if *self.open.borrow() {
            let _ = self.ws.send_with_u8_array(data);
        } else {
            // Not open yet — buffer; flushed on `onopen`.
            self.outbox.borrow_mut().push(data.to_vec());
        }
    }

    fn try_recv_from_server(&self) -> Option<Packet> {
        self.inbound.borrow_mut().pop_front()
    }

    fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }
}

/// Read `window.AXENSTAX_DEDICATED_WS` (set by the Docker-served page) and
/// resolve it to a WebSocket URL. A relative path like `"/ws"` resolves against
/// the page origin → `wss://host/ws` (matching the Caddy front); an absolute
/// `ws://`/`wss://` value is returned as-is. `None` when the global is absent
/// (the normal PWA), so this is inert for the solo web client.
pub fn dedicated_server_url() -> Option<String> {
    let window = web_sys::window()?;
    let val = js_sys::Reflect::get(&window, &JsValue::from_str("AXENSTAX_DEDICATED_WS")).ok()?;
    let s = val.as_string()?;
    if s.is_empty() {
        return None;
    }
    if s.starts_with("ws://") || s.starts_with("wss://") {
        return Some(s);
    }
    let loc = window.location();
    let proto = loc.protocol().ok()?; // "https:" / "http:"
    let host = loc.host().ok()?; // "host:port"
    let scheme = if proto == "https:" { "wss" } else { "ws" };
    let path = if s.starts_with('/') { s } else { format!("/{s}") };
    Some(format!("{scheme}://{host}{path}"))
}

/// Read the showcase / kiosk flags the Docker-served page sets as JS globals
/// (`window.AXENSTAX_SHOWCASE` etc.), mirroring `dedicated_server_url`'s read of
/// `AXENSTAX_DEDICATED_WS`. Absent globals ⇒ the default (disabled) config, so
/// this is inert for the normal PWA. Spec 2026-06-19 §8 Phase 2.
pub fn showcase_config() -> crate::showcase::ShowcaseConfig {
    let read = |key: &str| -> String {
        web_sys::window()
            .and_then(|w| js_sys::Reflect::get(&w, &JsValue::from_str(key)).ok())
            .and_then(|v| v.as_string())
            .unwrap_or_default()
    };
    crate::showcase::ShowcaseConfig::from_flags(
        &read("AXENSTAX_SHOWCASE"),
        &read("AXENSTAX_EXIT_ACTION"),
        &read("AXENSTAX_AUTO_LOOP_SECS"),
    )
}

/// Open a WebSocket to `url` (e.g. `wss://host:8443/ws`). Returns immediately;
/// the handshake completes asynchronously and queued sends flush on open.
pub fn connect_ws(url: &str) -> Result<WebSocketClientTransport, String> {
    let ws = WebSocket::new(url).map_err(|e| format!("WebSocket::new({url}) failed: {e:?}"))?;
    ws.set_binary_type(BinaryType::Arraybuffer);

    let inbound: Rc<RefCell<VecDeque<Packet>>> = Rc::new(RefCell::new(VecDeque::new()));
    let open = Rc::new(RefCell::new(false));
    let closed = Rc::new(RefCell::new(false));
    let outbox: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));

    // onmessage: arraybuffer → Vec<u8> → inbound queue.
    let on_message = {
        let inbound = inbound.clone();
        Closure::wrap(Box::new(move |e: MessageEvent| {
            if let Ok(buf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                let arr = js_sys::Uint8Array::new(&buf);
                inbound.borrow_mut().push_back(arr.to_vec());
            }
        }) as Box<dyn FnMut(MessageEvent)>)
    };
    ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    // onopen: mark open, flush anything buffered before the socket was ready.
    let on_open = {
        let ws2 = ws.clone();
        let open = open.clone();
        let outbox = outbox.clone();
        Closure::wrap(Box::new(move || {
            *open.borrow_mut() = true;
            for msg in outbox.borrow_mut().drain(..) {
                let _ = ws2.send_with_u8_array(&msg);
            }
            log::info!("WebSocket open");
        }) as Box<dyn FnMut()>)
    };
    ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));

    let on_error = Closure::wrap(Box::new(move |_e: web_sys::Event| {
        log::warn!("WebSocket error");
    }) as Box<dyn FnMut(web_sys::Event)>);
    ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));

    let on_close = {
        let open = open.clone();
        let closed = closed.clone();
        Closure::wrap(Box::new(move |_e: web_sys::Event| {
            *open.borrow_mut() = false;
            *closed.borrow_mut() = true;
            log::info!("WebSocket closed");
        }) as Box<dyn FnMut(web_sys::Event)>)
    };
    ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));

    log::info!("Opening WebSocket to {url}");
    Ok(WebSocketClientTransport {
        ws,
        inbound,
        open,
        closed,
        outbox,
        _on_message: on_message,
        _on_open: on_open,
        _on_error: on_error,
        _on_close: on_close,
    })
}
