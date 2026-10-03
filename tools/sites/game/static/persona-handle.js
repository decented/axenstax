// Axe'n'Stax — persona handle fetch (Signet kind-31000 display-name credential).
//
// Fetches a Signet persona's published display-name credential from a Nostr
// relay and returns the handle. Used by:
//   - lobby (display the persona handle on the signed-in card)
//   - /play/ menu (show the player their own handle)
//   - Future multiplayer JoinRequest construction (attach credential to
//     handshake). When ported to Rust (engine) / Python (server) the logic
//     below — pubkey filter, display-name tag lookup, expiry + signature
//     check, newest-wins — is the reference.
//
// Entry point:
//
//   window.AxeHandle.fetchPersonaHandle(pubkey, {
//       relayUrls?: string[], // default: signet-app's default relay set
//                             //   (trotters + nos.lol + damus + nostr.band +
//                             //    primal + ditto) — fanned out, newest-valid wins
//       relayUrl?: string,    // override: query ONLY this one relay (back-compat)
//       timeoutMs?: number,   // default: 3000
//       verify?: boolean,     // default: true — Schnorr + event-id check
//   }): Promise<{ handle, credentialId, expires, pubkey, rawEvent } | null>
//
// The persona handle is the Signet kind-31000 display-name credential, which
// signet-app publishes across its DEFAULT RELAY SET (not just our primary). A
// single-relay read can miss it, so the default is a multi-relay fan-out that
// merges results and picks the newest valid credential.
//
// Returns null on: timeout, relay error, no matching credential, all
// credentials expired, or signature verification failed (when verify:true).
//
// Dependencies:
//   window.AxeNoble.schnorr.verify        (from static/noble-curves.js)
//   crypto.subtle.digest (SubtleCrypto)   (HTTPS / localhost only)

(function () {
    'use strict';

    const DEFAULT_TIMEOUT_MS = 3000;
    const HEX_64 = /^[0-9a-f]{64}$/i;
    const HEX_128 = /^[0-9a-f]{128}$/i;

    function lower64hex(s) {
        return typeof s === 'string' && HEX_64.test(s) ? s.toLowerCase() : null;
    }

    function hexToBytes(hex) {
        const n = hex.length / 2;
        const bytes = new Uint8Array(n);
        for (let i = 0; i < n; i++) {
            bytes[i] = parseInt(hex.substr(i * 2, 2), 16);
        }
        return bytes;
    }

    function bytesToHex(bytes) {
        const out = new Array(bytes.length);
        for (let i = 0; i < bytes.length; i++) {
            out[i] = bytes[i].toString(16).padStart(2, '0');
        }
        return out.join('');
    }

    async function sha256Hex(text) {
        const buf = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
        return bytesToHex(new Uint8Array(buf));
    }

    function canonicalSerialise(event) {
        return JSON.stringify([0, event.pubkey, event.created_at, event.kind, event.tags, event.content]);
    }

    function getTag(tags, name) {
        if (!Array.isArray(tags)) return null;
        for (const t of tags) {
            if (Array.isArray(t) && t[0] === name && typeof t[1] === 'string') {
                return t[1];
            }
        }
        return null;
    }

    function hasValidStructure(e) {
        return !!(
            e && typeof e === 'object'
            && typeof e.id === 'string' && HEX_64.test(e.id)
            && typeof e.pubkey === 'string' && HEX_64.test(e.pubkey)
            && typeof e.sig === 'string' && HEX_128.test(e.sig)
            && typeof e.created_at === 'number'
            && e.kind === 31000
            && Array.isArray(e.tags)
            && typeof e.content === 'string'
        );
    }

    async function verifyEventFull(e, schnorrVerify) {
        if (!hasValidStructure(e)) return false;
        const expectedId = await sha256Hex(canonicalSerialise(e));
        if (expectedId.toLowerCase() !== e.id.toLowerCase()) return false;
        try {
            return schnorrVerify(hexToBytes(e.sig), hexToBytes(e.id), hexToBytes(e.pubkey));
        } catch (_err) {
            return false;
        }
    }

    function getRelayUrl() {
        const meta = document.querySelector('meta[name="axenstax-relay"]');
        if (meta && typeof meta.content === 'string' && meta.content.startsWith('wss://')) {
            return meta.content;
        }
        return 'wss://relay.trotters.cc';
    }

    // signet-app's default relay set (its src/lib/relay-service.ts: the primary
    // trotters.cc + PUBLIC_DEFAULT_RELAYS). signet-app publishes a persona's
    // kind-31000 display-name credential across THIS whole set, so a single-relay
    // read can miss it (the credential may not have reached trotters, or only
    // landed on the public relays). We fan the read out across all of them and
    // take newest-valid. nostr.band is an indexer (read-only on signet's side) —
    // fine to read from.
    const SIGNET_DEFAULT_RELAYS = [
        'wss://relay.trotters.cc',
        'wss://nos.lol',
        'wss://relay.damus.io',
        'wss://relay.nostr.band',
        'wss://relay.primal.net',
        'wss://relay.ditto.pub',
    ];

    // The relay set to query. An explicit opts.relayUrl (string) or opts.relayUrls
    // (array) overrides — back-compat / targeted reads; otherwise the configured
    // primary (<meta axenstax-relay> or trotters) is unioned with signet-app's
    // default set, deduped.
    function resolveRelays(opts) {
        if (opts.relayUrl) return [opts.relayUrl];
        if (Array.isArray(opts.relayUrls) && opts.relayUrls.length) return opts.relayUrls.slice();
        const set = [getRelayUrl()];
        for (const r of SIGNET_DEFAULT_RELAYS) {
            if (set.indexOf(r) === -1) set.push(r);
        }
        return set;
    }

    async function pickBestCredential(events, requirePubkey, verify) {
        const now = Math.floor(Date.now() / 1000);
        const schnorrVerify = (window.AxeNoble && window.AxeNoble.schnorr && window.AxeNoble.schnorr.verify) || null;
        if (verify && !schnorrVerify) {
            console.warn('AxeHandle: Schnorr verify unavailable — rejecting all events');
            return null;
        }
        const candidates = [];
        for (const e of events) {
            if (!hasValidStructure(e)) continue;
            if (e.pubkey.toLowerCase() !== requirePubkey) continue;
            const name = getTag(e.tags, 'display-name');
            if (!name) continue;
            // NIP-40 tag is `expiration` (signet-app emits `['expiration', '<unix>']`
            // via buildCredentialEvent). Reading `expires` here was a silent fail-open:
            // the tag never matched, so the `expires < now` check below was skipped and
            // expired credentials were accepted. Do NOT change back to `expires`.
            const expiresStr = getTag(e.tags, 'expiration');
            const expires = expiresStr !== null ? parseInt(expiresStr, 10) : null;
            if (expires !== null && Number.isFinite(expires) && expires < now) continue;
            if (verify) {
                // eslint-disable-next-line no-await-in-loop
                if (!(await verifyEventFull(e, schnorrVerify))) continue;
            }
            candidates.push({
                raw: e,
                name: name.slice(0, 100),  // cap defensively
                expires: Number.isFinite(expires) ? expires : 0,
                createdAt: e.created_at,
            });
        }
        if (candidates.length === 0) return null;
        candidates.sort((a, b) => b.createdAt - a.createdAt);
        const best = candidates[0];
        return {
            handle: best.name,
            credentialId: best.raw.id,
            expires: best.expires,
            pubkey: best.raw.pubkey,
            rawEvent: best.raw,
        };
    }

    function fetchPersonaHandle(pubkey, opts) {
        const lower = lower64hex(pubkey);
        if (!lower) return Promise.resolve(null);
        opts = opts || {};
        const relays = resolveRelays(opts);
        const timeoutMs = Number.isFinite(opts.timeoutMs) ? opts.timeoutMs : DEFAULT_TIMEOUT_MS;
        const verify = opts.verify !== false;

        return new Promise(function (resolve) {
            if (relays.length === 0) return resolve(null);
            const events = [];
            const sockets = [];
            let pending = relays.length;
            let settled = false;

            async function finish() {
                if (settled) return;
                settled = true;
                clearTimeout(timer);
                for (let i = 0; i < sockets.length; i++) {
                    try { sockets[i].close(); } catch (_) { /* ignore */ }
                }
                // Merge events from EVERY relay, then pick the single newest valid
                // credential across the union (signature + expiry checked inside).
                const best = await pickBestCredential(events, lower, verify);
                resolve(best);
            }

            // Each relay reports "done" (EOSE / close / error) at most once. When
            // every relay has, decide early instead of waiting out the timeout.
            function oneRelayDone() {
                pending -= 1;
                if (pending <= 0) finish();
            }

            const timer = setTimeout(finish, timeoutMs);

            relays.forEach(function (relayUrl) {
                let ws;
                let done = false;
                const subId = 'axh-' + Math.random().toString(36).slice(2, 10);
                function markDone() {
                    if (done) return;
                    done = true;
                    oneRelayDone();
                }
                try {
                    ws = new WebSocket(relayUrl);
                } catch (e) {
                    console.warn('AxeHandle: WebSocket construct failed for', relayUrl, e);
                    markDone();
                    return;
                }
                sockets.push(ws);

                ws.addEventListener('open', function () {
                    try {
                        ws.send(JSON.stringify(['REQ', subId, {
                            kinds: [31000],
                            authors: [lower],
                            limit: 20,
                        }]));
                    } catch (e) {
                        markDone();
                    }
                });

                ws.addEventListener('message', function (evt) {
                    let msg;
                    try { msg = JSON.parse(typeof evt.data === 'string' ? evt.data : ''); }
                    catch (_) { return; }
                    if (!Array.isArray(msg)) return;
                    if (msg[0] === 'EVENT' && msg[1] === subId && typeof msg[2] === 'object') {
                        events.push(msg[2]);
                    } else if ((msg[0] === 'EOSE' || msg[0] === 'CLOSED') && msg[1] === subId) {
                        markDone();
                    }
                });

                // An error alone doesn't end the read — buffered events may still
                // be useful and other relays continue; close/timeout settles it.
                ws.addEventListener('error', function () { markDone(); });
                ws.addEventListener('close', function () { markDone(); });
            });
        });
    }

    window.AxeHandle = { fetchPersonaHandle: fetchPersonaHandle };
})();
