// signet-debug-monitor v2 — drop-in via:
//   const s=document.createElement('script');s.src='/static/_debug-monitor.js';document.head.appendChild(s);
//
// Strategy: wrap window.WebSocket. Every new WS to a relay URL gets:
//   - send() wrapper that captures the REQ filter (so we learn the sessionPubkey
//     signet-verify subscribed for, without needing to monkey-patch the
//     getter-only Signet.waitForAuthResponse).
//   - addEventListener('message', …) tap that logs every kind-1059 EVENT
//     delivered to signet-verify's WS. addEventListener fires alongside
//     signet-verify's onmessage handler — does not interfere.
//
// Verdict after ≥1 event arrives or after the lobby's 130s countdown:
//   ZERO events arrived → phone-side mysignet.app is the bug (different relay /
//                         different #p tag / not publishing at all). File upstream.
//   Events DID arrive    → signet-verify silent-drops at one of its 12+ unlogged
//                         validation gates. Most likely: rumor kind != 29999, or
//                         NIP-44 v1↔v2 mismatch. Investigate inner-event format.

(function () {
    'use strict';

    if (window.__signetMonInstalled) {
        console.log('%c[mon] already installed — refresh the QR to retake snapshot',
            'color:#d4a044');
        return;
    }
    window.__signetMonInstalled = true;

    const OrigWebSocket = window.WebSocket;
    window.__lastSignetEvent = null;
    window.__lastSessionPub = null;

    // Survive post-auth location.reload() by persisting evidence to localStorage.
    // After reload, read with: JSON.parse(localStorage.__signetMonLog).
    function logToStorage(entry) {
        try {
            const arr = JSON.parse(localStorage.getItem('__signetMonLog') || '[]');
            arr.push(Object.assign({ t: Date.now() }, entry));
            // cap at 50 entries to bound storage
            if (arr.length > 50) arr.splice(0, arr.length - 50);
            localStorage.setItem('__signetMonLog', JSON.stringify(arr));
        } catch (_) { /* localStorage may be full or disabled */ }
    }
    function clearStorageLog() {
        try { localStorage.removeItem('__signetMonLog'); } catch (_) {}
    }
    // Fresh install → fresh log.
    clearStorageLog();
    logToStorage({ kind: 'install', url: location.href, ua: navigator.userAgent });
    console.log('%c[mon] persisting evidence to localStorage.__signetMonLog (survives reload)',
        'color:#9aa3b8');
    console.log('  After reload, read with: JSON.parse(localStorage.__signetMonLog)');

    function MonitoredWebSocket(url, protocols) {
        const ws = protocols !== undefined
            ? new OrigWebSocket(url, protocols)
            : new OrigWebSocket(url);

        // Only instrument relay traffic.
        if (typeof url !== 'string' || !/relay/i.test(url)) {
            return ws;
        }

        let sessionPub = null;
        let subId = null;
        let eosed = false;
        let eventCount = 0;
        let verdictPrinted = false;
        const startedAt = Date.now();
        let verdictTimer = null;

        console.log('%c[mon] relay WS created → ' + url,
            'color:#d4a044;font-weight:bold');

        const origSend = ws.send.bind(ws);
        ws.send = function (data) {
            try {
                const msg = JSON.parse(data);
                if (Array.isArray(msg) && msg[0] === 'REQ') {
                    const filt = msg[2] || {};
                    const pTags = filt['#p'] || [];
                    const kinds = filt.kinds || [];
                    if (kinds.indexOf(1059) >= 0 && pTags.length > 0) {
                        sessionPub = (pTags[0] || '').toLowerCase();
                        subId = msg[1];
                        console.log('%c[mon] signet-verify REQ captured',
                            'color:#d4a044;font-weight:bold');
                        console.log('  subId:           ' + subId);
                        console.log('  kinds:           ' + JSON.stringify(kinds));
                        console.log('  #p (sessionPub): ' + sessionPub);
                        console.log('  since:           ' + (filt.since || '(none)'));
                        window.__lastSessionPub = sessionPub;
                        logToStorage({
                            kind: 'req',
                            url: url,
                            subId: subId,
                            kinds: kinds,
                            sessionPub: sessionPub,
                            since: filt.since,
                        });
                    } else {
                        console.log('[mon] REQ (other):', msg);
                    }
                } else if (Array.isArray(msg) && msg[0] === 'CLOSE') {
                    console.log('[mon] CLOSE:', msg);
                }
            } catch (_) { /* ignore */ }
            return origSend(data);
        };

        ws.addEventListener('message', function (ev) {
            let msg;
            try {
                msg = JSON.parse(typeof ev.data === 'string' ? ev.data : '');
            } catch (_) { return; }
            if (!Array.isArray(msg)) return;
            const t = msg[0];
            if (t === 'EOSE' && msg[1] === subId) {
                if (!eosed) {
                    eosed = true;
                    console.log('[mon] EOSE — subscription live; waiting for phone publishes (auto-verdict in 130s)…');
                }
            } else if (t === 'EVENT' && msg[1] === subId) {
                const e = msg[2] || {};
                if (e.kind !== 1059) return; // shouldn't happen for this filter
                eventCount++;
                const pVals = (e.tags || [])
                    .filter(function (x) { return Array.isArray(x) && x[0] === 'p'; })
                    .map(function (x) { return (x[1] || '').toLowerCase(); });
                const ourMatch = sessionPub && pVals.indexOf(sessionPub) >= 0;
                const ageS = Math.floor(Date.now() / 1000) - (e.created_at || 0);
                console.log('%c[mon] ★★★ kind-1059 EVENT RECEIVED #' + eventCount + ' ★★★',
                    'color:#6dd57a;font-weight:bold;font-size:1.05em');
                console.log('  id:              ' + e.id);
                console.log('  wrap_pubkey:     ' + e.pubkey);
                console.log('  kind:            ' + e.kind + ' ✓');
                console.log('  created_at:      ' + e.created_at + '  (' + ageS + 's ago)');
                console.log('  tags:           ', e.tags);
                console.log('  p_tag values:   ', pVals);
                console.log('  matches our #p?: ' + (ourMatch ? '✓ yes' : '✗ NO (sessionPub=' + sessionPub + ')'));
                console.log('  content_length:  ' + (e.content || '').length + ' (NIP-44 ciphertext)');
                window.__lastSignetEvent = e;
                console.log('%c  → window.__lastSignetEvent + window.__lastSessionPub set',
                    'color:#9aa3b8');
                logToStorage({
                    kind: 'event',
                    n: eventCount,
                    id: e.id,
                    wrap_pubkey: e.pubkey,
                    event_kind: e.kind,
                    created_at: e.created_at,
                    age_s: ageS,
                    tags: e.tags,
                    p_tag_values: pVals,
                    matches_session_pub: ourMatch,
                    content_length: (e.content || '').length,
                    // Save full encrypted content too — for offline NIP-44 decrypt.
                    content_b64: e.content,
                });
                if (eventCount === 1) {
                    if (verdictTimer) clearTimeout(verdictTimer);
                    setTimeout(emitVerdict, 1500); // brief grace for any duplicate frames
                }
            } else if (t === 'NOTICE') {
                console.warn('[mon] NOTICE:', msg[1]);
            } else if (t === 'CLOSED' && msg[1] === subId) {
                console.warn('[mon] CLOSED:', msg.slice(2));
            }
        });

        function emitVerdict() {
            if (verdictPrinted) return;
            verdictPrinted = true;
            const elapsedS = Math.floor((Date.now() - startedAt) / 1000);
            console.log('---');
            if (eventCount === 0) {
                console.warn('%c[mon] VERDICT after ' + elapsedS + 's: ZERO kind-1059 events arrived for sessionPub=' + (sessionPub || '?').slice(0, 16) + '…',
                    'color:#e07070;font-weight:bold;font-size:1.1em');
                console.log('Phone is NOT publishing wraps tagged for our sessionPubkey on ' + url + '. Possible causes:');
                console.log('  (a) phone publishes to a different relay (mysignet.app no longer honours QR relay= param)');
                console.log('  (b) phone publishes with a different #p tag (wrong recipient pubkey)');
                console.log('  (c) phone never publishes at all (silent local failure on mysignet.app)');
                console.log('Action: bug is upstream. Pre-filing checklist now complete; tracked upstream, not yet fixed.');
                logToStorage({ kind: 'verdict', verdict: 'zero_events', elapsed_s: elapsedS });
            } else {
                console.warn('%c[mon] VERDICT after ' + elapsedS + 's: ' + eventCount + ' kind-1059 event(s) DID arrive',
                    'color:#d4a044;font-weight:bold;font-size:1.1em');
                console.log('Events ARE reaching the consumer subscription.');
                console.log('If the lobby reloaded soon after, signet-verify accepted the event and sign-in succeeded — the relay path is working.');
                console.log('If the lobby did NOT reload, signet-verify silent-dropped (most likely: rumor kind != 29999).');
                console.log('Either way, the EVENT is preserved at localStorage.__signetMonLog (kind=event entries) for offline decrypt.');
                logToStorage({ kind: 'verdict', verdict: 'events_arrived', count: eventCount, elapsed_s: elapsedS });
            }
        }
        verdictTimer = setTimeout(emitVerdict, 130000);

        return ws;
    }

    // Preserve constants and prototype so consumers can still do
    // `WebSocket.OPEN`, `instanceof WebSocket`, etc.
    MonitoredWebSocket.CONNECTING = OrigWebSocket.CONNECTING;
    MonitoredWebSocket.OPEN = OrigWebSocket.OPEN;
    MonitoredWebSocket.CLOSING = OrigWebSocket.CLOSING;
    MonitoredWebSocket.CLOSED = OrigWebSocket.CLOSED;
    MonitoredWebSocket.prototype = OrigWebSocket.prototype;

    window.WebSocket = MonitoredWebSocket;

    console.log('%c[mon] WebSocket interceptor installed',
        'color:#6dd57a;font-weight:bold;font-size:1.05em');
    console.log('  Click the QR Refresh button to trigger a fresh sign-in with monitoring active.');
    console.log('  After scanning the QR, watch this console for ★★★ EVENT RECEIVED ★★★ logs and the final VERDICT.');
})();
