// Registers the Axe'n'Stax service worker (Spec 32 Phase B — offline launch)
// AND auto-updates the page when a new build deploys.
//
// Auto-update path: app.py stamps the current build id into /sw.js, so every
// deploy changes the SW's bytes. The browser installs the new SW (it calls
// skipWaiting + clients.claim), which fires `controllerchange` — we reload once
// so the user lands on the fresh build with no manual hard-reload.
//
// In a separate file (not inline) so it passes the site's strict CSP (script-src 'self').
(function () {
    'use strict';
    if (!('serviceWorker' in navigator)) return;

    // Only auto-reload for UPDATES, never the first install, and never in a loop:
    //   - hadController: false on a first-ever visit (nothing to update FROM), so
    //     the initial install's controllerchange is ignored — the page already
    //     loaded fresh over the network.
    //   - reloading: guards against a double reload within one page load.
    var hadController = !!navigator.serviceWorker.controller;
    var reloading = false;
    navigator.serviceWorker.addEventListener('controllerchange', function () {
        if (reloading || !hadController) return;
        reloading = true;
        window.location.reload();
    });

    window.addEventListener('load', function () {
        navigator.serviceWorker.register('/sw.js').then(function (reg) {
            // Check for a new SW now and periodically while the tab is open, so a
            // fresh deploy is picked up automatically (no manual refresh). The
            // build id is stable per build, so this can't loop.
            var check = function () { try { reg.update(); } catch (_) { /* ignore */ } };
            check();
            setInterval(check, 60000);
        }).catch(function (e) {
            console.warn('SW registration failed:', (e && e.message) ? e.message : e);
        });
    });
})();
