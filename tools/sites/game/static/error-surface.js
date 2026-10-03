// Global error surface — shown on-screen and reported to the server.
// Kept small so it loads before the WASM bundle does anything interesting.
(function () {
    'use strict';

    function showError(msg) {
        const el = document.getElementById('errors');
        if (el) {
            el.style.display = 'block';
            // Build with text nodes instead of innerHTML so error strings
            // containing attacker-influenced text (e.g. a thrown exception
            // message that echoes a URL) can't become DOM-based XSS.
            el.appendChild(document.createTextNode(String(msg)));
            el.appendChild(document.createElement('br'));
        }
    }

    window.addEventListener('error', (e) => showError('ERROR: ' + e.message));
    window.addEventListener('unhandledrejection', (e) => {
        // Benign, expected browser behaviour: re-acquiring Pointer Lock without a
        // fresh user gesture rejects with NotAllowedError. Pressing Esc to leave
        // fullscreen releases the lock, and the engine's next cursor-grab (winit
        // calls `requestPointerLock` fire-and-forget, ignoring the promise) lands
        // here as an "unhandled" rejection. The game is unaffected — the cursor
        // re-locks on the next click — so don't pin a permanent error banner for
        // it (2026-06-16 report: "the message does not go away"). Real errors
        // still surface.
        const msg = String((e.reason && e.reason.message) || e.reason || '');
        if (/pointer\s*lock/i.test(msg)) { e.preventDefault(); return; }
        showError('REJECT: ' + e.reason);
    });

    const origLog = console.log;
    const origError = console.error;
    console.log = function () {
        origLog.apply(console, arguments);
        const msg = Array.from(arguments).join(' ');
        const el = document.getElementById('status');
        if (el && (msg.includes('auth:') || msg.includes('WASM') || msg.includes('GPU'))) {
            el.textContent = msg.replace(/^.*?INFO\s*/, '');
        }
    };
    console.error = function () {
        origError.apply(console, arguments);
        showError(Array.from(arguments).join(' '));
    };
})();
