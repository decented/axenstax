// Axe'n'Stax service worker — offline launch (Spec 32 Phase B).
//
// Conservative strategy (designed to never break the online path):
//   - /auth/* and /api/*        → network-only, NEVER cached or served from cache
//                                 (caching auth/feedback would freeze sign-in state).
//   - navigations (`/`)         → network-first → cached shell on network failure.
//                                 `/` is now the game itself (restructure 2026-06-27);
//                                 `/game` 301-redirects to `/` so it's never a shell.
//                                 Only 200s are cached.
//   - other same-origin GETs    → cache-first (static JS/CSS + the content-hashed WASM
//     (static + hashed WASM)      bundle), populated on first fetch.
//
// Why no precache list: the WASM filenames are content-hashed (axenstax-engine-<hash>),
// so they change every build. Runtime cache-first means a new build → new names →
// cache miss → fetched + cached; and because navigations are network-first, an online
// load always pulls the fresh index.html (with the new hashes). That is the update
// path (Spec 32 Phase 9) — no stale-bundle trap. Bumping CACHE forces a full refresh.
//
// Offline (airplane mode): navigation → network throws → cached `/` shell served →
// it references the last-cached hashed assets → served from cache → engine boots.

// The cache name carries the current build id (the content-hashed engine bundle),
// injected by app.py's /sw.js route (the `__BUILD_ID__` placeholder). So every
// deploy changes this SW's bytes → the browser installs a new SW → `activate`
// evicts the old build's cache → `sw-register.js` reloads once to the fresh
// build. That is the auto-update path (no manual hard-reload needed). In dev /
// unstamped serving the placeholder stays literal — harmless, just a fixed name.
//
// CACHE_REV is the manual bump: static JS under /static is cached cache-first, and
// the build id only changes when the wasm does. Bump CACHE_REV whenever /static
// changes without a wasm change, so activate() evicts the old cache and clients
// stop serving deleted/stale JS. rev 2 (2026-10-05, T0-4): the Stash/relay/Beacon
// bundles and trotters meta were removed from the page.
const CACHE_REV = 2;
const CACHE = 'axenstax-r' + CACHE_REV + '-__BUILD_ID__';

self.addEventListener('install', (event) => {
    // Precache the navigation shell (fixed path — no hashed names) so a cold
    // offline launch has something to serve. `/` is the game shell (it boots the
    // WASM engine); it references the content-hashed assets, runtime-cached on the
    // first online load. (`/game` is no longer a shell — it 301-redirects to `/`.)
    event.waitUntil((async () => {
        const c = await caches.open(CACHE);
        // Game shell at the root — always a 200, safe to cache directly.
        try { await c.add('/'); } catch (_) { /* ignore */ }
        await self.skipWaiting();
    })());
});

self.addEventListener('activate', (event) => {
    event.waitUntil((async () => {
        const keys = await caches.keys();
        await Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)));
        await self.clients.claim();
    })());
});

self.addEventListener('fetch', (event) => {
    const req = event.request;
    if (req.method !== 'GET') return;                       // never touch POST (auth/verify, logout, feedback)
    const url = new URL(req.url);
    if (url.origin !== self.location.origin) return;        // only same-origin (relay/mysignet untouched)
    if (url.pathname.startsWith('/auth/') || url.pathname.startsWith('/api/')) return;  // network-only

    if (req.mode === 'navigate') {
        event.respondWith((async () => {
            // Cached shell to fall back to: this exact page, else the root game
            // shell. (Both precached on install / runtime-cached.)
            const cachedShell = async () =>
                (await caches.match(req))
                || (await caches.match('/'))
                || Response.error();

            // Offline fast-path: don't even touch the network when the browser
            // already knows it's offline — this is what stops the freeze on a
            // dead LAN route (a fetch to an unreachable IP can hang on a SYN
            // timeout instead of failing fast, leaving the page stuck loading).
            if (!self.navigator.onLine) {
                return cachedShell();
            }

            // Online (or unsure): network-first, but RACE it against a short
            // timeout so a slow-to-fail network can never freeze the launch —
            // if the network doesn't answer quickly, serve the cached shell.
            try {
                const res = await Promise.race([
                    fetch(req),
                    new Promise((_, reject) =>
                        setTimeout(() => reject(new Error('nav-timeout')), 3000)),
                ]);
                if (res && res.ok) {                        // cache only 200s (not the signed-out 302)
                    const c = await caches.open(CACHE);
                    c.put(req, res.clone());
                }
                return res;
            } catch (_) {
                return cachedShell();
            }
        })());
        return;
    }

    // Assets. Two strategies:
    //   - Immutable, content-hashed engine bundle (filename changes every build)
    //     → cache-first: serve from cache instantly, never re-download.
    //   - Everything else (mutable /static JS/CSS, manifest, icons) → network-first
    //     so code changes actually propagate online (cache-first would pin the old
    //     copy forever), with the same offline fast-path + timeout as navigations
    //     so it never hangs and still works offline.
    const immutable =
        url.pathname.startsWith('/axenstax-engine-') || url.pathname.endsWith('.wasm');
    event.respondWith((async () => {
        const cached = await caches.match(req);
        if (immutable && cached) return cached;

        if (!self.navigator.onLine) {
            return cached || Response.error();
        }
        try {
            const res = await Promise.race([
                fetch(req),
                new Promise((_, reject) =>
                    setTimeout(() => reject(new Error('asset-timeout')), 3000)),
            ]);
            if (res && res.ok) {
                const c = await caches.open(CACHE);
                c.put(req, res.clone());
            }
            return res;
        } catch (_) {
            return cached || Response.error();
        }
    })());
});
