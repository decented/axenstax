#!/usr/bin/env python3
"""Axe'n'Stax — game site (port 8094).

Serves the PWA runtime: `/` IS the WASM game (login-free local-sandbox tier since
2026-06-27; the legacy `/game` path 301-redirects to `/`). The browser build has
no feedback channel (lobby mailbox + /api/test-board removed 2026-10-01).
Marketing/landing and the spec website are separate apps under
tools/sites/marketing/ and tools/sites/docs/.

No login: the web build is a local sandbox / taster — anonymous (guest) play is
the default. Identity, cloud saves and multiplayer live in the native download.
"""

import asyncio
import base64
import hashlib
import json
import logging
import os
import re
import time
from pathlib import Path

import httpx
from dotenv import load_dotenv
from fastapi import FastAPI, HTTPException, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse, HTMLResponse, JSONResponse, RedirectResponse, Response
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates

load_dotenv()
log = logging.getLogger(__name__)


class _AuthQueryRedactFilter(logging.Filter):
    """Redact query strings from /auth/* access log lines.

    Why: Signet sign-in redirects land on /auth/callback with the response in
    URL params — pubkey, signature, eventId, display_name, and (per the age-
    attestation spec) a future age= boolean ladder. Uvicorn's AccessFormatter
    writes the full request line including the query string to stdout / the
    site log. Browser-side leak vectors are already closed (the success
    template carries `<meta name="referrer" content="no-referrer">` and the
    redirect uses `location.replace` so the callback URL doesn't stick in
    browser history). This closes the last hole — the server-side access log.
    """
    _AUTH_PREFIX = "/auth/"

    def filter(self, record: logging.LogRecord) -> bool:
        args = getattr(record, "args", None)
        # uvicorn.logging.AccessFormatter unpacks args as
        #   (client_addr, method, full_path, http_version, status_code)
        if not (isinstance(args, tuple) and len(args) == 5):
            return True
        client_addr, method, full_path, http_version, status_code = args
        if isinstance(full_path, str) and full_path.startswith(self._AUTH_PREFIX) and "?" in full_path:
            base = full_path.split("?", 1)[0]
            record.args = (client_addr, method, f"{base}?[REDACTED]", http_version, status_code)
        return True


logging.getLogger("uvicorn.access").addFilter(_AuthQueryRedactFilter())


class _McSkinRedactFilter(logging.Filter):
    """Strip the query string AND client IP from /mc-skin access-log lines so
    neither a player's Minecraft username/UUID nor their IP address is written
    to disk (spec §11, no-log rule)."""

    def filter(self, record: logging.LogRecord) -> bool:
        args = getattr(record, "args", None)
        # uvicorn.logging.AccessFormatter unpacks args as
        #   (client_addr, method, full_path, http_version, status_code)
        if not (isinstance(args, tuple) and len(args) == 5):
            return True
        client_addr, method, full_path, http_version, status_code = args
        if isinstance(full_path, str) and full_path.startswith("/mc-skin"):
            # Redact both the query string (name/uuid) and the client address (IP).
            redacted_path = full_path.split("?", 1)[0] + "?[REDACTED]" if "?" in full_path else full_path
            record.args = ("-", method, redacted_path, http_version, status_code)
        return True


logging.getLogger("uvicorn.access").addFilter(_McSkinRedactFilter())

BASE_DIR = Path(__file__).parent
# tools/sites/game → AxeNStax repo root
PROJECT_ROOT = BASE_DIR.parent.parent.parent
WASM_DIST = PROJECT_ROOT / "game" / "engine" / "dist"
# The in-browser crash reporter (/api/wasm-error) was removed 2026-10-03 — no
# player-typed or crash text is stored server-side. Delete any log it left.
(PROJECT_ROOT / "wasm-errors.log").unlink(missing_ok=True)
CERTS_DIR = BASE_DIR / "certs"

PORT = int(os.environ.get("PORT", "8094"))
DOCS_URL = os.environ.get("DOCS_URL", "https://localhost:8095")
MARKETING_URL = os.environ.get("MARKETING_URL", "https://localhost:8096")

# --- No cloud, no relay, no third party — the taster is OFFLINE (T0-4) ---
# The web build talks to its own origin only: CSP `connect-src 'self'`, no Blossom
# / Stash / Nostr relay / Beacon code on the page (2026-10-05; before that cloud
# save was already hard-off here from the 2026-09-27 audit). Cloud save, sharing
# and multiplayer live in the native app. tools/smoke/forbidden-symbol.mjs and
# tests/test_offline_taster.py pin this so it cannot silently regrow.


app = FastAPI(title="Axe'n'Stax — Game")

# --- Security headers ---
# /play/ overrides this baseline with a stricter CSP that pins inline-script
# hashes (Trunk's loader).
#
# 2026-10-05 (T0-4): the web taster is fully offline — `connect-src 'self'`, so
# the page can reach only its own origin (the /mc-skin proxy, /static, the WASM
# bundle). The old `wss:` allowance existed for NIP-46 bunker sign-in and the
# Nostr save manifest; both are gone from the web build. `/` serves the stricter
# per-build CSP below (`_build_play_headers`), which also carries
# 'wasm-unsafe-eval' so the WASM engine can instantiate.
_BASELINE_CSP = (
    "default-src 'self'; "
    "script-src 'self'; "
    "style-src 'self' 'unsafe-inline'; "
    "connect-src 'self'; "
    "img-src 'self' data:; "
    "font-src 'self' data:; "
    "media-src 'self'; "
    "worker-src 'self'; "
    "manifest-src 'self'; "
    "object-src 'none'; "
    "base-uri 'self'; "
    "form-action 'self'; "
    "frame-src 'none'; "
    "frame-ancestors 'none'"
)


@app.middleware("http")
async def _add_security_headers(request, call_next):
    response = await call_next(request)
    response.headers.setdefault("X-Frame-Options", "DENY")
    response.headers.setdefault("X-Content-Type-Options", "nosniff")
    response.headers.setdefault("Referrer-Policy", "no-referrer")
    scheme = request.url.scheme or ""
    forwarded = request.headers.get("x-forwarded-proto", "").lower()
    if scheme == "https" or forwarded == "https":
        response.headers.setdefault(
            "Strict-Transport-Security", "max-age=31536000; includeSubDomains"
        )
    ctype = response.headers.get("content-type", "").lower()
    if "text/html" in ctype:
        response.headers.setdefault("Content-Security-Policy", _BASELINE_CSP)
    return response


_cors_origins = os.environ.get("CORS_ORIGINS", "").split(",")
_cors_origins = [o.strip() for o in _cors_origins if o.strip()]
if not _cors_origins:
    # Local dev default. For LAN testing (phone/tablet on the same network) set
    # CORS_ORIGINS="https://<lan-ip>:8094,http://<lan-ip>:8094,https://localhost:8094".
    _cors_origins = [
        f"http://localhost:{PORT}",
        f"https://localhost:{PORT}",
    ]
app.add_middleware(
    CORSMiddleware,
    allow_origins=_cors_origins,
    allow_methods=["GET", "POST"],
    allow_headers=["Content-Type", "X-Requested-With", "X-Admin-Token"],
)
app.mount("/static", StaticFiles(directory=str(BASE_DIR / "static")), name="static")
templates = Jinja2Templates(directory=str(BASE_DIR / "templates"))

# --- Routes ---
#
# 2026-09-27 audit fix: the game site used to mount a Signet `/auth/*` router
# (challenge/verify/whoami/logout, minting a 90-day identity cookie) plus a
# voice-feedback `/api/feedback` proxy. Both were already dead — auth.js hasn't
# made an /auth round-trip since the 2026-06-27 web-local-sandbox rewrite (it
# boots straight into an anonymous guest session), and /api/feedback always
# 503'd once the voice server was decommissioned (VOICE_SERVER_ORIGIN empty).
# Live but unreachable-by-design code on an "anonymous, no-cookies" taster is
# exactly the gap CLAUDE.md's red line 3 warns about, so both are removed
# outright rather than left dormant. `tools/sites/game/auth.py` (the router
# module + cookie HMAC machinery) is deleted with this change; the CONSOLE site
# has its own separate auth.py copy for its real operator login and is
# untouched. `game/engine/src/wasm_auth.rs::sign_join_via_js` still looks up
# `window.__axenstax_sign_auth_event` for the Phase-4 join-auth signing bridge,
# but that hook was never defined by any page script even before this change
# (grepped: no definer anywhere in tools/sites/game/static/*.js) — it already
# degrades to "sign hook missing (sign in first)" and a guest join. No Rust
# change needed; this is confirmed pre-existing dead code, not something this
# fix newly breaks.


# --- Minecraft skin import proxy (spec §8.4 / §11) ---
# Hard rules: no logging of name/uuid/ip, no disk storage, in-memory cache only
# (TTL 300 s), Mojang-only upstream, 4 s/call + 10 s chain timeout, 10 req/IP/60 s.
_MC_SKIN_CACHE: dict[str, dict] = {}          # key -> {uuid, name, slim, png, cached_at}
_MC_SKIN_CACHE_TTL = 300.0
_MC_SKIN_CACHE_MAX = 512  # max total entries (name keys + uuid: aliases combined)
_MC_SKIN_RATE: dict[str, tuple[int, float]] = {}  # ip -> (count, window_start)
_MC_SKIN_RATE_MAX = 10
_MC_SKIN_RATE_WINDOW = 60.0
_MC_CALL_TIMEOUT = 4.0
_MC_CHAIN_DEADLINE = 10.0
_MC_HOST_NAME = "https://api.mojang.com"
_MC_HOST_SESSION = "https://sessionserver.mojang.com"
_MC_HOST_TEXTURE = "textures.minecraft.net"
_MC_USERNAME_RE = re.compile(r"^[A-Za-z0-9_]{1,16}$")
_MC_UUID_RE = re.compile(r"^[0-9a-fA-F]{32}$")
# Trusted reverse-proxy peer address.  Behind nginx the app listens on loopback
# so EVERY request arrives from 127.0.0.1; we trust XFF only from that peer.
# NOTE: nginx must set `proxy_set_header X-Forwarded-For $remote_addr;` (or
# equivalent) — this is an infra item owned by the deploy maintainer.
_MC_TRUSTED_PROXY = "127.0.0.1"


def _mc_real_ip(request: Request) -> str:
    """Derive the real client IP for rate-limiting.

    Trusts X-Forwarded-For ONLY when the immediate peer is the trusted loopback
    proxy (_MC_TRUSTED_PROXY).  Falls back to request.client.host for local
    (no-proxy) runs so development / tests are unaffected.
    """
    peer = request.client.host if request.client else None
    if peer == _MC_TRUSTED_PROXY:
        xff = request.headers.get("x-forwarded-for", "").split(",")[0].strip()
        if xff:
            return xff
    return peer or "unknown"


def _mc_rate_allow(ip: str) -> bool:
    now = time.monotonic()
    count, start = _MC_SKIN_RATE.get(ip, (0, now))
    if now - start >= _MC_SKIN_RATE_WINDOW:
        count, start = 0, now
    if count >= _MC_SKIN_RATE_MAX:
        return False
    _MC_SKIN_RATE[ip] = (count + 1, start)
    if len(_MC_SKIN_RATE) > 4096:  # lazy prune
        for k, (_c, s) in list(_MC_SKIN_RATE.items()):
            if now - s >= _MC_SKIN_RATE_WINDOW:
                _MC_SKIN_RATE.pop(k, None)
    return True


def _mc_cache_get(key: str):
    e = _MC_SKIN_CACHE.get(key)
    if e and (time.monotonic() - e["cached_at"]) < _MC_SKIN_CACHE_TTL:
        return e
    if e:
        _MC_SKIN_CACHE.pop(key, None)  # lazy evict
    return None


def _mc_cache_evict() -> None:
    """Evict oldest entries when the cache exceeds _MC_SKIN_CACHE_MAX.

    Each successful lookup adds two entries (a name key + a ``uuid:`` alias),
    so eviction removes name keys first and then also drops the corresponding
    ``uuid:`` alias to keep the two in sync.
    """
    if len(_MC_SKIN_CACHE) <= _MC_SKIN_CACHE_MAX:
        return
    # Sort all current entries by insertion time (oldest first).
    sorted_items = sorted(_MC_SKIN_CACHE.items(), key=lambda kv: kv[1]["cached_at"])
    to_remove = len(_MC_SKIN_CACHE) - _MC_SKIN_CACHE_MAX
    removed = 0
    for key, entry in sorted_items:
        if removed >= to_remove:
            break
        if key not in _MC_SKIN_CACHE:
            continue  # already evicted (e.g. uuid: alias of a name key removed above)
        _MC_SKIN_CACHE.pop(key, None)
        removed += 1
        if not key.startswith("uuid:"):
            # Also evict the paired uuid: alias so the two stay in sync.
            uuid_key = "uuid:" + entry["uuid"].lower()
            if _MC_SKIN_CACHE.pop(uuid_key, None) is not None:
                removed += 1


def _mc_decode_textures(profile: dict):
    """profile (sessionserver JSON) → (skin_url, slim). Raises ValueError with a
    code on no-custom-skin. Never logs."""
    props = profile.get("properties") or []
    blob_b64 = None
    for p in props:
        if p.get("name") == "textures":
            blob_b64 = p.get("value")
            break
    if not blob_b64:
        raise ValueError("no_custom_skin")
    tex = json.loads(base64.b64decode(blob_b64))
    skin = (tex.get("textures") or {}).get("SKIN")
    if not skin or not skin.get("url"):
        raise ValueError("no_custom_skin")
    url = skin["url"]
    slim = ((skin.get("metadata") or {}).get("model") == "slim")
    return url, slim


@app.get("/mc-skin")
async def mc_skin(request: Request, name: str | None = None, uuid: str | None = None):
    """Server-side relay to Mojang for the web skin importer (browsers can't call
    Mojang — no CORS). UUID-first: refresh passes ?uuid= to skip the rate-limited
    name endpoint. No logging, no storage, in-memory cache only (spec §11)."""
    ip = _mc_real_ip(request)
    if not _mc_rate_allow(ip):
        return JSONResponse({"error": "rate_limited"}, status_code=429)

    # Validate inputs (never echo them back).
    if uuid is not None:
        if not _MC_UUID_RE.match(uuid):
            return JSONResponse({"error": "bad_name"}, status_code=400)
        cache_key = "uuid:" + uuid.lower()
    elif name is not None:
        if not _MC_USERNAME_RE.match(name):
            return JSONResponse({"error": "bad_name"}, status_code=400)
        cache_key = name.lower()
    else:
        return JSONResponse({"error": "bad_name"}, status_code=400)

    cached = _mc_cache_get(cache_key)
    if cached:
        return Response(
            content=cached["png"],
            media_type="image/png",
            headers={
                "X-Mc-Uuid": cached["uuid"],
                "X-Mc-Name": cached["name"],
                "X-Mc-Slim": "1" if cached["slim"] else "0",
                "Cache-Control": "no-store",
            },
        )

    deadline = time.monotonic() + _MC_CHAIN_DEADLINE
    try:
        async with httpx.AsyncClient() as client:
            # Step 1 (skip on refresh): username → UUID.
            if uuid is None:
                call_timeout = min(_MC_CALL_TIMEOUT, max(0.1, deadline - time.monotonic()))
                r1 = await client.get(
                    f"{_MC_HOST_NAME}/users/profiles/minecraft/{name}",
                    timeout=call_timeout,
                )
                if r1.status_code == 404:
                    return JSONResponse({"error": "not_found"}, status_code=404)
                if r1.status_code == 429:
                    return JSONResponse({"error": "rate_limited"}, status_code=429)
                if r1.status_code != 200:
                    return JSONResponse({"error": "offline"}, status_code=502)
                resolved_uuid = r1.json().get("id")
                if not resolved_uuid:
                    return JSONResponse({"error": "not_found"}, status_code=404)
            else:
                resolved_uuid = uuid

            if time.monotonic() > deadline:
                return JSONResponse({"error": "offline"}, status_code=502)

            # Step 2: UUID → profile (textures blob).
            call_timeout = min(_MC_CALL_TIMEOUT, max(0.1, deadline - time.monotonic()))
            r2 = await client.get(
                f"{_MC_HOST_SESSION}/session/minecraft/profile/{resolved_uuid}",
                timeout=call_timeout,
            )
            if r2.status_code == 204:
                return JSONResponse({"error": "not_found"}, status_code=404)
            if r2.status_code == 429:
                return JSONResponse({"error": "rate_limited"}, status_code=429)
            if r2.status_code != 200:
                return JSONResponse({"error": "offline"}, status_code=502)
            profile = r2.json()
            canonical_name = profile.get("name", name or "Player")
            try:
                skin_url, slim = _mc_decode_textures(profile)
            except ValueError as e:
                code = str(e) if str(e) in ("no_custom_skin",) else "offline"
                status = 422 if code == "no_custom_skin" else 502
                return JSONResponse({"error": code}, status_code=status)

            # Allowlist: PNG must be on Mojang's texture CDN (spec §11 #4).
            if not skin_url.split("://", 1)[-1].split("/", 1)[0].lower() == _MC_HOST_TEXTURE:
                return JSONResponse({"error": "offline"}, status_code=502)

            if time.monotonic() > deadline:
                return JSONResponse({"error": "offline"}, status_code=502)

            # Step 3: fetch the PNG (https form works even though Mojang gives http).
            png_url = skin_url.replace("http://", "https://", 1)
            call_timeout = min(_MC_CALL_TIMEOUT, max(0.1, deadline - time.monotonic()))
            r3 = await client.get(png_url, timeout=call_timeout)
            if (
                r3.status_code != 200
                or r3.headers.get("content-type", "").split(";")[0].strip() != "image/png"
            ):
                return JSONResponse({"error": "offline"}, status_code=502)
            png = r3.content
    except (httpx.TimeoutException, httpx.RequestError, ValueError, KeyError):
        # ValueError covers json.JSONDecodeError (non-JSON / empty Mojang body).
        # KeyError covers unexpected profile shapes.
        return JSONResponse({"error": "offline"}, status_code=502)

    entry = {
        "uuid": resolved_uuid,
        "name": canonical_name,
        "slim": slim,
        "png": png,
        "cached_at": time.monotonic(),
    }
    _MC_SKIN_CACHE[cache_key] = entry
    # Also index by uuid so a follow-up ?uuid= refresh hits cache too.
    _MC_SKIN_CACHE["uuid:" + resolved_uuid.lower()] = entry
    _mc_cache_evict()  # enforce size cap after every write

    return Response(
        content=png,
        media_type="image/png",
        headers={
            "X-Mc-Uuid": resolved_uuid,
            "X-Mc-Name": canonical_name,
            "X-Mc-Slim": "1" if slim else "0",
            "Cache-Control": "no-store",
        },
    )


# --- /play/ gate + WASM serving ---
_INLINE_SCRIPT_RE = re.compile(
    r"<script(?P<attrs>[^>]*)>(?P<body>.*?)</script>",
    re.DOTALL | re.IGNORECASE,
)
_SRC_ATTR_RE = re.compile(r"\bsrc\s*=", re.IGNORECASE)


def _inline_script_hashes(index_html: str) -> list[str]:
    out: list[str] = []
    for m in _INLINE_SCRIPT_RE.finditer(index_html):
        attrs = m.group("attrs") or ""
        if _SRC_ATTR_RE.search(attrs):
            continue
        body = m.group("body")
        digest = hashlib.sha256(body.encode("utf-8")).digest()
        out.append("'sha256-" + base64.b64encode(digest).decode("ascii") + "'")
    return out


def _build_play_headers() -> dict:
    index_path = WASM_DIST / "index.html"
    hash_tokens: list[str] = []
    if index_path.exists():
        try:
            hash_tokens = _inline_script_hashes(index_path.read_text())
        except OSError as e:
            log.warning(f"CSP hash compute failed: {e}")
    base_tokens = ["'self'", "'wasm-unsafe-eval'"]
    script_src = " ".join([*base_tokens, *hash_tokens])
    csp = (
        "default-src 'self'; "
        f"script-src {script_src}; "
        "style-src 'self' 'unsafe-inline'; "
        "connect-src 'self'; "
        "img-src 'self' data:; "
        "font-src 'self' data:; "
        "media-src 'self'; "
        "worker-src 'self'; "
        "manifest-src 'self'; "
        "object-src 'none'; "
        "base-uri 'self'; "
        "form-action 'self'; "
        "frame-src 'none'; "
        "frame-ancestors 'none'"
    )
    return {
        "Content-Security-Policy": csp,
        "X-Frame-Options": "DENY",
        "Referrer-Policy": "no-referrer",
        "X-Content-Type-Options": "nosniff",
        "Cache-Control": "no-cache, must-revalidate",
    }


@app.get("/", response_class=HTMLResponse)
async def entrance(request: Request):
    """`/` serves the WASM game bundle directly (restructure 2026-06-27:
    play.axenstax.com root IS the game; the separate sign-in entrance is retired).

    Web login was retired (2026-06-27) — the web build is a login-free local
    sandbox / taster, so there is no sign-in stop. Anonymous (guest) play is the
    default and needs no account; identity, cloud saves (Stash) and multiplayer
    live in the free desktop app. The legacy `/game` path 301-redirects here for
    bookmarks/PWA/external links. Web tier = anonymous local sandbox per
    docs/superpowers/specs/2026-06-27-web-local-sandbox-design.md."""
    if not WASM_DIST.exists():
        return HTMLResponse(
            "<h1>WASM build not found</h1>"
            "<p>Run <code>trunk build</code> in <code>game/engine/</code> on the host first.</p>",
            status_code=503,
        )

    index_html = (WASM_DIST / "index.html").read_text()
    # The in-game lobby "Exit" leaves to the marketing home — the real "home" now
    # that / IS the game. Inject it so the engine needn't hardcode the host (dev
    # vs prod). auth.js reads <meta name="marketing-url">.
    index_html = re.sub(
        r'<meta name="marketing-url" content="[^"]*">',
        f'<meta name="marketing-url" content="{MARKETING_URL}">',
        index_html,
        count=1,
    )
    return HTMLResponse(
        content=index_html,
        status_code=200,
        headers=_build_play_headers(),
    )


@app.get("/game")
async def game_gate():
    """Legacy path kept for bookmarks, installed-PWA cached start_url, search-
    engine-indexed URLs, and external links. The game now lives at `/` (the root
    of play.axenstax.com), so `/game` permanently redirects there."""
    return RedirectResponse(url="/", status_code=301)


# --- Help / cert helper (lives on game so PWA users can grab it inline) ---
@app.get("/help/cert.pem")
async def download_cert():
    cert_path = CERTS_DIR / "cert.pem"
    if not cert_path.exists():
        raise HTTPException(status_code=404, detail="Certificate not generated yet")
    return FileResponse(
        path=str(cert_path),
        filename="axenstax-cert.pem",
        media_type="application/x-pem-file",
    )


# --- Desktop-app funnel ---
# The in-game lobby (engine menu.rs) shows a "Cloud saves & multiplayer live in
# the free desktop app →" nudge on web. It opens a same-origin /download so the
# engine needn't know the docs host; we redirect to the docs download page, which
# adapts to the environment via DOCS_URL. Web is a local sandbox; the power
# features live in the native build.
@app.get("/download")
async def download_redirect():
    return RedirectResponse(url=f"{DOCS_URL}/download", status_code=302)


# --- PWA: service worker + manifest (Spec 32 Phase B) ---
# Both served at root so the SW controls "/" scope (covering the root-mounted WASM).
# Registered before the catch-all mount so they win precedence.
def _current_build_id() -> str:
    """Stable id for the current WASM build — the content-hashed engine bundle
    filename, which trunk regenerates every build. Stamped into the service
    worker so the SW's bytes change each deploy: the browser only re-evaluates a
    service worker whose bytes differ, so this is what makes a new build
    auto-install, evict the old cache, and (with sw-register.js) auto-reload.
    MUST be stable within a build (same across requests) or it would loop — the
    content hash is. Falls back to the dist mtime, then a constant, in dev."""
    try:
        names = sorted(p.name for p in WASM_DIST.glob("axenstax-engine-*_bg.wasm"))
        if names:
            return names[0].removesuffix(".wasm")
    except Exception:
        pass
    try:
        return str(int((WASM_DIST / "index.html").stat().st_mtime))
    except Exception:
        return "dev"


@app.get("/sw.js")
async def service_worker():
    sw_path = BASE_DIR / "static" / "sw.js"
    if not sw_path.exists():
        raise HTTPException(status_code=404, detail="service worker not found")
    # Stamp the current build id in so the SW's bytes change every deploy — that
    # is the signal the browser uses to install the new SW, evict the old cache,
    # and auto-reload to the fresh build. Without it the SW is byte-identical
    # between builds and never updates (the stale-bundle trap).
    sw_src = sw_path.read_text().replace("__BUILD_ID__", _current_build_id())
    return Response(
        content=sw_src,
        media_type="text/javascript",
        headers={
            # Root scope even though it could be served elsewhere; and never let the
            # browser serve a stale SW from HTTP cache — SW updates must be picked up.
            "Service-Worker-Allowed": "/",
            "Cache-Control": "no-cache, must-revalidate",
        },
    )


@app.get("/manifest.webmanifest")
async def web_manifest():
    m_path = BASE_DIR / "static" / "manifest.webmanifest"
    if not m_path.exists():
        raise HTTPException(status_code=404, detail="manifest not found")
    return FileResponse(path=str(m_path), media_type="application/manifest+json")


# WASM dist mount — registered LAST so explicit routes (/, /auth/*, /api/*,
# /admin/*, /static/*, /help/*) all win precedence over the catch-all. The
# mount serves the trunk-built hashed assets (axenstax-engine-<hash>.js,
# axenstax-engine-<hash>_bg.wasm) at the paths the rebuilt index.html
# references — see game/engine/Trunk.toml `public_url = "/"`.
if WASM_DIST.exists():
    app.mount("/", StaticFiles(directory=str(WASM_DIST), html=False), name="wasm_assets")


if __name__ == "__main__":
    import uvicorn
    cert = CERTS_DIR / "cert.pem"
    key = CERTS_DIR / "key.pem"
    ssl_kwargs = {}
    if cert.exists() and key.exists():
        ssl_kwargs = {"ssl_certfile": str(cert), "ssl_keyfile": str(key)}
    # proxy_headers=True + forwarded_allow_ips: in production the app listens on
    # loopback behind nginx, so every peer is 127.0.0.1.  Uvicorn will promote
    # X-Forwarded-For / X-Real-IP only when the peer matches the allowlist.
    # _mc_real_ip() then picks up the validated left-most XFF entry for rate
    # limiting (held in memory only, never logged or written) so each user gets their own 10-req/60s bucket instead of all
    # collapsing into one.  Local (no-proxy) runs are unaffected — when there is
    # no XFF, _mc_real_ip() falls back to request.client.host.
    uvicorn.run(
        app,
        host="0.0.0.0",
        port=PORT,
        log_level="info",
        # No access log: it writes every client IP + request line. We do not
        # log IP addresses (see /privacy). The redact filters above stay as
        # belt-and-braces for anyone launching `uvicorn app:app` directly.
        access_log=False,
        proxy_headers=True,
        forwarded_allow_ips="127.0.0.1",
        **ssl_kwargs,
    )
