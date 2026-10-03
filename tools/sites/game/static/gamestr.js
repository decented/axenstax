// AxeNStax gamestr glue — opt-in leaderboard score publish (NIP-133, kind 33334).
// Sibling of beacon.js. ALL wire concerns live here; the engine only triggers
// the offer at a scenario's not-ended -> ended edge (game/engine/src/gamestr.rs).
//
// Design: docs/superpowers/specs/2026-06-11-gamestr-wasm-scoring-design.md
//
// Privacy invariant (standing rule: identity defaults NON-public): NOTHING is
// signed or published unless the player ticks the default-OFF box. A published
// score is signed by the current signed-in npub, so the opt-in is the guard.
//
// Score integrity is HONOUR-SYSTEM: a client-signed kind-33334 event is
// forgeable. Fine for a staffed booth; server-validated proof-of-play is the
// long-run answer (see the spec).
//
// Browser surface (wasm-bindgen extern target the engine calls):
//   window.axenstax_gamestr_offer(gameId, work) -> shows the opt-in overlay
//     (always shown for an in-scope scenario; the bunker is connected lazily on Post)
// Node-test surface: module.exports = { buildScoreEvent, buildHandleEvent, publishScore }.
(function () {
  'use strict';

  var SCORE_KIND = 33334; // NIP-133 game score — parameterized-replaceable
  var PROFILE_KIND = 0; // minimal kind-0 carrying just the opt-in handle
  var DEFAULT_RELAY = 'wss://relay.trotters.cc';
  // gamestr's OWN board relays — verified from gamestr.io's deployed bundle
  // (2026-06-12). The score must land on a relay the board reads or it never shows;
  // the original default of publishing ONLY to our relay was exactly why posts
  // didn't appear. We target gamestr's own infra and let its aggregator fan out to
  // the public relays — we deliberately do NOT seed nos.lol/damus/primal ourselves
  // (a child's name shouldn't be blasted to public infra by us; identity defaults
  // non-public). Override at deploy time via <meta name="gamestr-relay">.
  var GAMESTR_RELAYS = ['wss://relay.gamestr.io', 'wss://test.gamestr.io'];
  var LS_CONSENT = 'axenstax.gamestr.consent'; // '1' | '0'
  var LS_HANDLE = 'axenstax.gamestr.handle';
  var OVERLAY_FLAG = '__axenstax_gamestr_overlay_open';
  var OVERLAY_ID = 'axenstax-gamestr-overlay';

  function nowSec() {
    return Math.floor(Date.now() / 1000);
  }

  // --- pure event builders (unit-tested) -------------------------------------
  function buildScoreEvent(gameId, work) {
    return {
      kind: SCORE_KIND,
      created_at: nowSec(),
      tags: [['d', String(gameId)]],
      // content is a PLAIN STRINGIFIED NUMBER — the NIP-133 default and the form
      // every live gamestr leaderboard uses (verified 2026-06-12: 2048-nostrapps,
      // asteroids-nostrapps, melrise all post a bare number). Hash Dash's score IS
      // a single number (work, higher = better), so it maps 1:1. We previously sent
      // the category-JSON form `{"work":N}`; the board couldn't render it.
      content: String(Math.max(0, Math.floor(Number(work) || 0))),
    };
  }
  function buildHandleEvent(handle) {
    return {
      kind: PROFILE_KIND,
      created_at: nowSec(),
      tags: [],
      content: JSON.stringify({ name: String(handle) }),
    };
  }

  // --- consent-gated publish (unit-tested with a fake signer + relay) --------
  // opts:   { signer: { signEvent(template) }, relay: { publish(signedEvent) } }
  // params: { gameId, work, handle, consent }
  // Returns { published, kinds }. When consent is false NOTHING is signed or
  // published — the privacy invariant. When a handle is given, a minimal kind-0
  // is published first so the board shows a name; otherwise the row is the npub.
  async function publishScore(opts, params) {
    if (!params || !params.consent) return { published: false, kinds: [] };
    var signer = opts && opts.signer;
    var relay = opts && opts.relay;
    if (!signer || typeof signer.signEvent !== 'function') throw new Error('gamestr: no signer');
    if (!relay || typeof relay.publish !== 'function') throw new Error('gamestr: no relay');
    var kinds = [];
    var warnings = [];
    var handle = params.handle != null ? String(params.handle).trim() : '';
    // The kind-0 name is COSMETIC — it only labels the board row. A relay hiccup
    // on it must NOT block the score (the actual leaderboard entry), so publish it
    // BEST-EFFORT and carry on, recording a warning. (Previously a name-relay
    // failure threw and the score silently never posted.)
    if (handle) {
      try {
        var signedHandle = await signer.signEvent(buildHandleEvent(handle));
        await relay.publish(signedHandle);
        kinds.push(PROFILE_KIND);
      } catch (e) {
        warnings.push('name not updated: ' + ((e && e.message) || e));
      }
    }
    // The score IS the leaderboard entry — a failure here is a real failure and
    // propagates to the caller (surfaced in the overlay + console).
    var signedScore = await signer.signEvent(buildScoreEvent(params.gameId, params.work));
    await relay.publish(signedScore);
    kinds.push(SCORE_KIND);
    return { published: true, kinds: kinds, scoreId: signedScore.id, warnings: warnings };
  }

  // ===========================================================================
  // Browser glue below — DOM overlay + live signer/relay. The testable core is
  // above; everything here is wiring exercised by the live/manual test.
  // ===========================================================================

  function metaContent(name) {
    if (typeof document === 'undefined') return '';
    var m = document.querySelector('meta[name="' + name + '"]');
    return m ? m.content : '';
  }
  // Relays the gamestr board READS — where the score MUST land to appear on the
  // leaderboard. Comma-separated <meta name="gamestr-relay"> overrides the built-in
  // GAMESTR_RELAYS set.
  function boardRelays() {
    var meta = metaContent('gamestr-relay');
    if (meta) {
      return meta.split(',').map(function (s) { return s.trim(); }).filter(Boolean);
    }
    return GAMESTR_RELAYS.slice();
  }
  // Our OWN relay — kept in the publish set and used for the post-publish read-back
  // so a "posted but not on the board" report stays diagnosable on our infra.
  function verifyRelay() {
    return metaContent('axenstax-relay') || DEFAULT_RELAY;
  }
  // Every relay the score is published to: gamestr's board relays + our verify relay.
  function publishRelays() {
    var board = boardRelays();
    var verify = verifyRelay();
    return board.indexOf(verify) === -1 ? board.concat([verify]) : board;
  }
  function rawSigner() {
    return typeof window !== 'undefined' && typeof window.__axenstax_get_signer === 'function'
      ? window.__axenstax_get_signer()
      : null;
  }
  // The signed-in pubkey (hex), if any. Available even when the bunker isn't
  // connected: auth.js sets window.__axenstax_pubkey after a verified sign-in.
  // Falls back to the live signer's pubkey.
  function currentPubkey() {
    if (typeof window !== 'undefined' && typeof window.__axenstax_pubkey === 'string') {
      return window.__axenstax_pubkey;
    }
    var s = rawSigner();
    return s && typeof s.pubkey === 'string' ? s.pubkey : null;
  }
  // The handle the player SIGNED IN as — the Signet session display name the
  // entrance card shows (signet-login stores it at 'signet:login.displayName').
  // Synchronous and present the moment a signed-in player finishes a round, so
  // the leaderboard name field can default to it INSTANTLY. This is the SAME
  // canonical signed-in handle the feedback log uses (cloud.js signedInHandle) —
  // deliberately NOT a player-edited kind-0.
  function signedInHandle() {
    try {
      var v = (window.localStorage.getItem('signet:login.displayName') || '').trim();
      return v ? v.slice(0, 24) : null;
    } catch (_) {
      return null;
    }
  }
  // Async fallback prefill: the Signet kind-31000 display-name credential (same
  // source the entrance card treats as canonical), used when the synchronous
  // session name above isn't available. Read-only — nothing leaves the device;
  // the handle is only published if the player ticks consent and clicks Post. A
  // relay round-trip, so it lands after the overlay shows; `isEdited()` guards
  // against clobbering anything the player has started typing. The signed-in
  // persona is authoritative — it overwrites a remembered fallback already shown.
  function prefillPersonaHandle(handleEl, isEdited) {
    if (typeof window === 'undefined' || !window.AxeHandle ||
        typeof window.AxeHandle.fetchPersonaHandle !== 'function') {
      return;
    }
    var pubkey = currentPubkey();
    if (!pubkey) return;
    window.AxeHandle.fetchPersonaHandle(pubkey, { timeoutMs: 3500 })
      .then(function (info) {
        if (!info || !info.handle) return;
        if (handleEl && !(typeof isEdited === 'function' && isEdited())) {
          handleEl.value = String(info.handle).slice(0, 24);
        }
      })
      .catch(function () { /* prefill is best-effort */ });
  }
  function makeClient(urls) {
    if (typeof window === 'undefined' || !window.AxeRelay || !urls || !urls.length) return null;
    return window.AxeRelay.makeRelayClient(urls);
  }
  function liveRelay() {
    return makeClient(publishRelays());
  }
  function lsGet(k, dflt) {
    try {
      var v = window.localStorage.getItem(k);
      return v == null ? dflt : v;
    } catch (_) {
      return dflt;
    }
  }
  function lsSet(k, v) {
    try {
      window.localStorage.setItem(k, v);
    } catch (_) {
      /* private mode / disabled — non-fatal */
    }
  }

  // A usable signer right now, or null. (Bunker may not be connected yet.)
  function readySigner() {
    var s = rawSigner();
    return s && typeof s.signEvent === 'function' ? s : null;
  }

  // Acquire a usable signer, reconnecting the bunker if it isn't live yet. The
  // bunker often isn't connected at the round-end instant (a phone bunker's
  // background reconnect can take >15s, or be asleep), so Post is the explicit
  // user gesture that connects it — the SAME canonical path auth.js uses at boot
  // (window.Signet.restoreSession → window.__axenstax_set_signer). Bounded so
  // Post can't hang forever. Returns a signer or null.
  function ensureSigner(onStatus) {
    var s = readySigner();
    if (s) return Promise.resolve(s);
    var Signet = typeof window !== 'undefined' ? window.Signet : null;
    if (!Signet || typeof Signet.restoreSession !== 'function') {
      return Promise.resolve(null);
    }
    if (onStatus) onStatus('Connecting to your signer…');
    var timeout = new Promise(function (resolve) {
      setTimeout(function () { resolve('timeout'); }, 30000);
    });
    var reconnect = Signet.restoreSession()
      .then(function (restored) {
        var sig = restored && restored.signer ? restored.signer : restored;
        if (sig && typeof sig.signEvent === 'function') {
          // Share it with the rest of the app (cloud save, etc.).
          if (typeof window.__axenstax_set_signer === 'function') {
            window.__axenstax_set_signer(restored && restored.signer ? restored : sig);
          }
          return sig;
        }
        return null;
      })
      .catch(function () { return null; });
    return Promise.race([reconnect, timeout]).then(function (r) {
      // If we timed out, the reconnect may STILL be in flight — re-read in case
      // it landed; otherwise null.
      return r && r !== 'timeout' ? r : readySigner();
    });
  }

  function signerHint(e) {
    var msg = e && e.message ? e.message : String(e || '');
    if (/no signer|signer/i.test(msg) || !msg) {
      return "Couldn't reach your signer — make sure your signer app is awake and online, then try again.";
    }
    return "Couldn't post: " + msg;
  }

  function setOverlayFlag(open) {
    if (typeof window !== 'undefined') window[OVERLAY_FLAG] = !!open;
  }

  function closeOverlay() {
    var el = document.getElementById(OVERLAY_ID);
    if (el && el.parentNode) el.parentNode.removeChild(el);
    setOverlayFlag(false);
  }

  // Build + show the opt-in overlay. DOM-only; the publish itself goes through
  // the unit-tested publishScore() with the live signer + relay.
  function offer(gameId, work) {
    if (typeof document === 'undefined') return;
    if (document.getElementById(OVERLAY_ID)) return; // already showing
    // ALWAYS show the overlay — do NOT gate on a live signer. The bunker is
    // connected lazily at Post time (ensureSigner), so the player always gets
    // the chance to post even if their bunker is still waking up.

    var prevConsent = lsGet(LS_CONSENT, '0') === '1';

    var root = document.createElement('div');
    root.id = OVERLAY_ID;
    root.setAttribute('role', 'dialog');
    root.setAttribute('aria-modal', 'true');
    root.style.cssText =
      'position:fixed;inset:0;z-index:2147483600;display:flex;align-items:center;' +
      'justify-content:center;background:rgba(8,16,12,0.72);font-family:system-ui,' +
      'sans-serif;color:#e8f5ee;';

    var card = document.createElement('div');
    card.style.cssText =
      'background:#14241a;border:1px solid #2f5e44;border-radius:14px;padding:22px 24px;' +
      'width:min(92vw,380px);box-shadow:0 12px 40px rgba(0,0,0,0.5);';

    var title = document.createElement('div');
    title.textContent = 'Hash Dash complete';
    title.style.cssText = 'font-size:18px;font-weight:700;margin-bottom:4px;';

    var scoreLine = document.createElement('div');
    scoreLine.textContent = 'You did ' + Math.max(0, Math.floor(Number(work) || 0)) + ' work.';
    scoreLine.style.cssText = 'font-size:15px;opacity:0.92;margin-bottom:16px;';

    var consentRow = document.createElement('label');
    consentRow.style.cssText = 'display:flex;align-items:center;gap:9px;cursor:pointer;font-size:14px;margin-bottom:12px;';
    var consent = document.createElement('input');
    consent.type = 'checkbox';
    consent.checked = prevConsent;
    consent.style.cssText = 'width:17px;height:17px;accent-color:#3fae6f;';
    var consentText = document.createElement('span');
    consentText.textContent = 'Show my score on the leaderboard';
    consentRow.appendChild(consent);
    consentRow.appendChild(consentText);

    var handleWrap = document.createElement('div');
    handleWrap.style.cssText = 'margin-bottom:12px;';
    var handleLabel = document.createElement('div');
    handleLabel.textContent = 'Name to show (you can edit it)';
    handleLabel.style.cssText = 'font-size:12px;opacity:0.72;margin-bottom:5px;';
    var handle = document.createElement('input');
    handle.type = 'text';
    handle.maxLength = 24;
    handle.placeholder = 'e.g. AxoBuilder';
    handle.autocomplete = 'off';
    handle.style.cssText =
      'width:100%;box-sizing:border-box;padding:9px 11px;border-radius:8px;border:1px solid #2f5e44;' +
      'background:#0e1a13;color:#e8f5ee;font-size:14px;';
    handleWrap.appendChild(handleLabel);
    handleWrap.appendChild(handle);

    // The name field ALWAYS defaults to the handle you SIGNED IN with — every
    // game, even if you edited it last round. Edits are per-game and are NOT
    // remembered as the new default (that was the bug: a one-off edit stuck). A
    // remembered name is used ONLY as a fallback when there's no signed-in handle
    // at all (e.g. a guest who hasn't set one). Still fully editable before Post.
    var userEdited = false;
    handle.addEventListener('input', function () { userEdited = true; });
    var signedIn = signedInHandle();
    handle.value = signedIn || lsGet(LS_HANDLE, '');
    if (!signedIn) {
      prefillPersonaHandle(handle, function () { return userEdited; });
    }

    // Posting needs the signer awake. A phone bunker often isn't connected the
    // instant a round ends (it reconnects in the background), so nudge the player
    // to switch it on — shown only while "show me on the leaderboard" is ticked.
    var bunkerHint = document.createElement('div');
    bunkerHint.textContent = '⚡ Make sure your Signet bunker is on, so your score can post.';
    bunkerHint.style.cssText =
      'font-size:12px;line-height:1.35;color:#ffd9a0;opacity:0.92;margin-bottom:14px;';

    // Disable the handle field + hide the bunker nudge unless they want to be shown.
    function syncHandleEnabled() {
      var on = consent.checked;
      handle.disabled = !on;
      handleWrap.style.opacity = on ? '1' : '0.45';
      bunkerHint.style.display = on ? 'block' : 'none';
    }
    consent.addEventListener('change', syncHandleEnabled);
    syncHandleEnabled();

    var btnRow = document.createElement('div');
    btnRow.style.cssText = 'display:flex;gap:10px;justify-content:flex-end;';
    var notNow = document.createElement('button');
    notNow.textContent = 'Not now';
    notNow.style.cssText =
      'padding:9px 15px;border-radius:8px;border:1px solid #2f5e44;background:transparent;' +
      'color:#cfe6da;font-size:14px;cursor:pointer;';
    var post = document.createElement('button');
    post.textContent = 'Post';
    post.style.cssText =
      'padding:9px 18px;border-radius:8px;border:none;background:#3fae6f;color:#06150d;' +
      'font-size:14px;font-weight:700;cursor:pointer;';

    var status = document.createElement('div');
    status.style.cssText = 'font-size:12px;opacity:0.8;margin-top:12px;min-height:15px;';

    notNow.addEventListener('click', function () {
      lsSet(LS_CONSENT, '0');
      closeOverlay();
    });

    post.addEventListener('click', function () {
      var consented = consent.checked;
      var name = handle.value.trim();
      lsSet(LS_CONSENT, consented ? '1' : '0');
      lsSet(LS_HANDLE, name);
      if (!consented) {
        // Treated as "don't show me" — nothing leaves the device.
        closeOverlay();
        return;
      }
      post.disabled = true;
      notNow.disabled = true;
      status.textContent = 'Preparing…';
      var relayList = publishRelays(); // the relays we publish to — logged for diagnosis
      var liveClient = null;
      // Connect the bunker if needed, then publish. This is why Post can take a
      // moment the first time (the bunker wakes up); subsequent posts are instant.
      ensureSigner(function (m) { status.textContent = m; })
        .then(function (signer) {
          if (!signer) throw new Error('no signer');
          liveClient = liveRelay();
          if (!liveClient) throw new Error('no relay');
          status.textContent = 'Posting to the leaderboard…';
          console.log('[gamestr] publishing to [' + relayList.join(', ') + '] — game "' + gameId +
            '", work ' + work + (name ? ', name "' + name + '"' : ' (npub only)'));
          return publishScore(
            { signer: signer, relay: liveClient },
            { gameId: gameId, work: work, handle: name, consent: true }
          );
        })
        .then(function (res) {
          if (res && res.warnings && res.warnings.length) {
            console.warn('[gamestr] ' + res.warnings.join('; '));
          }
          console.log('[gamestr] published kinds ' + JSON.stringify((res && res.kinds) || []) +
            ', score event ' + ((res && res.scoreId) || '?'));
          // Best-effort READ-BACK from gamestr's OWN board relays so a "posted but
          // not on the board" report is diagnosable: if a gamestr relay returns our
          // score, it's on the board's infra (any remaining gap is gamestr-side
          // rendering/aggregation); if not, the gamestr relay never stored it.
          // Diagnostic only — never fails the post.
          var pubkey = currentPubkey();
          var boardClient = makeClient(boardRelays());
          if (res && res.scoreId && pubkey && boardClient && typeof boardClient.query === 'function') {
            boardClient.query(pubkey, SCORE_KIND, String(gameId))
              .then(function (evs) {
                var found = (evs || []).some(function (e) { return e.id === res.scoreId; });
                console.log('[gamestr] read-back from gamestr relays [' + boardRelays().join(', ') +
                  ']: score ' + res.scoreId +
                  (found ? ' CONFIRMED on a gamestr relay ✓ — should appear on the board'
                         : ' NOT found on gamestr relays — the relay dropped it'));
              })
              .catch(function (e) { console.warn('[gamestr] read-back query failed:', e); })
              .then(function () { try { boardClient.close(); } catch (_) { /* best-effort */ } });
          }
          status.textContent = (res && res.warnings && res.warnings.length)
            ? 'Posted ✓ (name not updated)'
            : 'Posted ✓';
          setTimeout(closeOverlay, 850);
        })
        .catch(function (e) {
          console.warn('[gamestr] post to [' + relayList.join(', ') + '] failed:', e);
          status.textContent = signerHint(e);
          post.disabled = false;
          notNow.disabled = false;
        });
    });

    btnRow.appendChild(notNow);
    btnRow.appendChild(post);
    card.appendChild(title);
    card.appendChild(scoreLine);
    card.appendChild(consentRow);
    card.appendChild(handleWrap);
    card.appendChild(bunkerHint);
    card.appendChild(btnRow);
    card.appendChild(status);
    root.appendChild(card);
    document.body.appendChild(root);
    setOverlayFlag(true); // engine gates ALL input while this is set
  }

  // --- exports ---------------------------------------------------------------
  var factory = {
    buildScoreEvent: buildScoreEvent,
    buildHandleEvent: buildHandleEvent,
    publishScore: publishScore,
  };
  if (typeof module !== 'undefined' && module.exports) module.exports = factory;
  if (typeof window !== 'undefined') {
    window.AxeGamestrFactory = factory;
    window.axenstax_gamestr_offer = function (gameId, work) {
      try {
        offer(gameId, work);
      } catch (e) {
        // Never let a leaderboard hiccup break the end-of-game flow.
        console.warn('gamestr offer failed', e);
        setOverlayFlag(false);
      }
      // The engine imports this as an ASYNC binding (gamestr.rs:
      // `async fn js_gamestr_offer(...) -> Result<JsValue, JsValue>`), so
      // wasm-bindgen does `JsFuture::from(returnValue)` = `returnValue.then(...)`.
      // Returning undefined here means `undefined.then(...)` → a hard crash
      // ("Cannot read properties of undefined (reading 'then')") the instant a
      // scenario ends (the not-ended->ended edge that fires offer_score). The
      // offer is genuinely fire-and-forget — the overlay is shown synchronously
      // above; the publish happens later on Post — so resolve immediately.
      return Promise.resolve();
    };
  }
})();
