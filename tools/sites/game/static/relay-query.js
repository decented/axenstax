// Axe'n'Stax — minimal Nostr relay READ bridge for npub→address resolution
// (Server Card / Spec A). The WASM engine has no `nostr` crate, so it calls this
// to read events from a relay.
//
//   window.axenstax_relay_query(relayUrl, filterJson) -> Promise<string[]>
//
// Opens a WebSocket to `relayUrl`, sends one REQ with the parsed `filterJson`
// filter, collects each EVENT's event-object as a JSON string until EOSE or a 5s
// timeout, then closes and resolves with the collected strings. Never rejects —
// resolves with whatever arrived (possibly []), so the Rust side degrades to a
// failed resolve (the host-pinned join path is unaffected).
//
// OWNER BOUNDARY: verified live in a browser against a real relay.
(function () {
  "use strict";

  window.axenstax_relay_query = function (relayUrl, filterJson) {
    return new Promise(function (resolve) {
      var out = [];
      var done = false;
      var ws = null;

      function finish() {
        if (done) return;
        done = true;
        try { if (ws) ws.close(); } catch (e) { /* ignore */ }
        resolve(out);
      }

      var timer = setTimeout(finish, 5000);

      var filter;
      try {
        filter = JSON.parse(filterJson);
      } catch (e) {
        clearTimeout(timer);
        resolve(out);
        return;
      }

      try {
        ws = new WebSocket(relayUrl);
      } catch (e) {
        clearTimeout(timer);
        resolve(out);
        return;
      }

      var sub = "axe-resolve-" + Math.random().toString(36).slice(2, 8);

      ws.onopen = function () {
        try {
          ws.send(JSON.stringify(["REQ", sub, filter]));
        } catch (e) {
          clearTimeout(timer);
          finish();
        }
      };

      ws.onmessage = function (ev) {
        var msg;
        try { msg = JSON.parse(ev.data); } catch (e) { return; }
        if (!Array.isArray(msg)) return;
        if (msg[0] === "EVENT" && msg[2]) {
          out.push(JSON.stringify(msg[2]));
        } else if (msg[0] === "EOSE") {
          clearTimeout(timer);
          finish();
        }
      };

      ws.onerror = function () { clearTimeout(timer); finish(); };
      ws.onclose = function () { clearTimeout(timer); finish(); };
    });
  };
})();
