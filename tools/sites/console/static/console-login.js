// Operator Console — sign-in flow (signet-login). Mirrors the game site's
// /auth/challenge → Signet.login → /auth/verify, minus the WASM boot.
(function () {
  "use strict";
  var APP_NAME = "Axe'n'Stax Operator Console";
  var relayMeta = document.querySelector('meta[name="axenstax-relay"]');
  // A public relay by default (no AxeNStax-operated relay is a default —
  // CLAUDE.md red line 2); the page's axenstax-relay meta can override it.
  var RELAY_URL = (relayMeta && relayMeta.content) || "wss://relay.damus.io";
  var statusEl = document.getElementById("status");

  function setStatus(m) {
    if (statusEl) statusEl.textContent = m;
  }

  async function postVerify(authEvent, displayName) {
    var payload = { authEvent: authEvent };
    if (displayName) payload.displayName = displayName;
    var res = await fetch("auth/verify", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Requested-With": "fetch" },
      body: JSON.stringify(payload),
    });
    if (!res.ok) {
      var detail = "";
      try { detail = (await res.json()).detail || ""; } catch (e) { /* ignore */ }
      throw new Error("verify " + res.status + (detail ? " — " + detail : ""));
    }
    // Cookie set. /dashboard enforces that the signed-in npub IS the operator.
    window.location.href = "dashboard";
  }

  var btn = document.getElementById("signin");
  if (btn) {
    btn.addEventListener("click", async function () {
      btn.disabled = true;
      setStatus("Starting sign-in…");
      try {
        if (!window.Signet || typeof window.Signet.login !== "function") {
          throw new Error("Signet SDK didn't load");
        }
        var chRes = await fetch("auth/challenge", {
          method: "POST",
          headers: { "X-Requested-With": "fetch" },
        });
        if (!chRes.ok) throw new Error("/auth/challenge " + chRes.status);
        var challenge = (await chRes.json()).challenge;
        setStatus("Scan the QR or approve on your signer…");
        var session = await window.Signet.login({
          appName: APP_NAME,
          challenge: challenge,
          relayUrl: RELAY_URL,
        });
        setStatus("Verifying…");
        await postVerify(session.authEvent, session.displayName);
      } catch (e) {
        setStatus("Sign-in failed: " + (e && e.message ? e.message : e));
        btn.disabled = false;
      }
    });
  }

  // Same-device redirect return.
  if (window.Signet && typeof window.Signet.handleRedirectCallback === "function") {
    window.Signet
      .handleRedirectCallback()
      .then(function (result) {
        if (result && result.authEvent) {
          setStatus("Verifying…");
          postVerify(result.authEvent, result.displayName).catch(function (e) {
            setStatus("Sign-in failed: " + (e && e.message ? e.message : e));
          });
        }
      })
      .catch(function () { /* no pending redirect */ });
  }
})();
