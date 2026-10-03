#!/usr/bin/env python3
"""Axe'n'Stax — engine specs / architecture website (port 8095).

The builder side of the project — serves docs/ specs, ADRs, roadmap, test sheets,
and the native / self-host binary download. Lives on docs.axenstax.org in
production. Player-facing content moved off this site: the guided journey is the
learn site (tools/sites/learn/), the dense reference is the wiki site
(tools/sites/wiki/). The PWA runtime lives on the game site (tools/sites/game/,
play.axenstax.com). Internal working docs are repo-only, no longer published here.
"""

import hashlib
import logging
import os
import re
from pathlib import Path

import markdown
from dotenv import load_dotenv
from fastapi import FastAPI, HTTPException, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse, HTMLResponse, JSONResponse
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates

from platforms import INSTALLER_META as _INSTALLER_META
from platforms import OS_LABELS as _OS_LABELS
from platforms import detect_os as _detect_os
from versioning import filter_newest_per_platform, latest_manifest

load_dotenv()
log = logging.getLogger(__name__)

BASE_DIR = Path(__file__).parent
PROJECT_ROOT = BASE_DIR.parent.parent.parent
DOCS_ROOT = PROJECT_ROOT / "docs"
BUILD_DIR = PROJECT_ROOT / "build" / "release"
BINARY_NAME = "axenstax-engine"
CERTS_DIR = BASE_DIR / "certs"

# Native installers (built by cargo-packager — see tools/packaging/). The CI workflow
# `gh run download`s artifacts here; `tools/packaging/build-local.sh` + a copy step do the
# same locally. Empty in a fresh checkout → the page gracefully falls back to build-from-source.
INSTALLERS_DIR = Path(
    os.environ.get("AXENSTAX_INSTALLERS_DIR", str(PROJECT_ROOT / "build" / "installers"))
)

# Installer table + OS labels live in platforms.py (pure, unit-tested by check.sh).
_INSTALLER_NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9 _.\-]{0,150}$")

# The published-version derivation lives in versioning.py (pure, no FastAPI) so
# `check.sh` can unit-test the /download/latest.json contract with a bare python3.

# The AppImage embeds an auto-update URL pointing the updater at /download/<file>
# (NOT the /installer/ route). The stable "latest" AppImage and its .zsync delta
# map are the only two files served by that bare /download/{name} route — anything
# else 404s, so it is not a general file server. The versioned AppImage stays the
# user-facing download; -latest- is the in-place upgrade channel.
# See docs/superpowers/specs/2026-06-18-appimage-auto-update-design.md.
_AUTOUPDATE_NAMES = {
    "axenstax-engine-latest-x86_64.AppImage",
    "axenstax-engine-latest-x86_64.AppImage.zsync",
}
_sha256_cache: dict[tuple, str] = {}

PORT = int(os.environ.get("PORT", "8095"))
GAME_URL = os.environ.get("GAME_URL", f"https://localhost:8094")
MARKETING_URL = os.environ.get("MARKETING_URL", f"https://localhost:8096")
# The open-source project home (axenstax.org). Was historically reached via the
# overloaded MARKETING_URL; now explicit so the "The project" link is unambiguous.
PROJECT_URL = os.environ.get("PROJECT_URL", "https://localhost:8099")

app = FastAPI(title="Axe'n'Stax — Specs & Docs")

# --- Security headers ---
# Voice server origin — env-driven; blank collapses the widget's CSP slots cleanly
# (see the game app for the full rationale). Local dev keeps the LAN VM default.
# (Was hardcoded http://192.168.1.10:4100.)
VOICE_SERVER_ORIGIN = os.environ.get("VOICE_SERVER_ORIGIN", "").rstrip("/")  # voice server removed; default off
_VOICE_WS = (
    VOICE_SERVER_ORIGIN.replace("https://", "wss://", 1).replace("http://", "ws://", 1)
    if VOICE_SERVER_ORIGIN
    else ""
)
_VOICE_SRC = f" {VOICE_SERVER_ORIGIN}" if VOICE_SERVER_ORIGIN else ""
_VOICE_CONNECT = f" {VOICE_SERVER_ORIGIN} {_VOICE_WS}" if VOICE_SERVER_ORIGIN else ""
_BASELINE_CSP = (
    "default-src 'self'; "
    f"script-src 'self'{_VOICE_SRC}; "
    "style-src 'self' 'unsafe-inline'; "
    f"connect-src 'self'{_VOICE_CONNECT}; "
    "img-src 'self' data:; "
    "font-src 'self' data:; "
    f"media-src 'self'{_VOICE_SRC}; "
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
    _cors_origins = [
        f"https://localhost:{PORT}",
        f"http://localhost:{PORT}",
    ]
app.add_middleware(
    CORSMiddleware,
    allow_origins=_cors_origins,
    allow_methods=["GET"],
    allow_headers=["Content-Type"],
)
app.mount("/static", StaticFiles(directory=str(BASE_DIR / "static")), name="static")
templates = Jinja2Templates(directory=str(BASE_DIR / "templates"))

# --- Markdown renderer ---
MD = markdown.Markdown(
    extensions=[
        "tables",
        "fenced_code",
        "toc",
        "attr_list",
        "pymdownx.tasklist",
    ],
    extension_configs={
        "pymdownx.tasklist": {"custom_checkbox": True},
    },
)


def render_md(path: Path) -> tuple[str, str]:
    MD.reset()
    text = path.read_text()
    html = MD.convert(text)
    toc = getattr(MD, "toc", "")
    return html, toc


# --- Document sections ---

SECTIONS = {
    "gameplay": {
        "title": "Gameplay",
        "subtitle": "Everything about how the game plays — Axolittle's territory",
        "docs": [
            {"path": "vision/platform-overview.md", "title": "Platform Overview", "short": "The vision"},
            {"path": "spec/05-gameplay-systems.md", "title": "Gameplay Systems", "short": "Movement, blocks, inventory, combat, mobs"},
        ],
    },
    "host": {
        "title": "Host a Server",
        "subtitle": "Run your own self-hostable server — your world, your rules",
        "docs": [
            {"path": "operators/host-overview.md", "title": "Host Your Own Server", "short": "What it is, why, and what you need — start here"},
            {"path": "operators/dedicated-server.md", "title": "Run Your Own Server", "short": "Self-host the dedicated server in Docker — install + run"},
            {"path": "operators/operator-console.md", "title": "Operator Console", "short": "Manage who can join, settings + privacy — no command line"},
            {"path": "operators/world-chat.md", "title": "World Chat", "short": "Turn on in-world chat for your server — tiers, Charter ceiling, rooms"},
        ],
    },
    "tech": {
        "title": "Tech & Architecture",
        "subtitle": "Engine, networking, infrastructure — Staxolottle's territory",
        "docs": [
            {"path": "architecture/ADR-001-full-custom-engine.md", "title": "ADR-001: Full Custom Engine", "short": "Why we build from scratch"},
            {"path": "architecture/ADR-002-tech-stack.md", "title": "ADR-002: Tech Stack", "short": "Rust, wgpu, Lightning, hosting"},
            {"path": "architecture/ADR-003-pwa-first-for-alpha.md", "title": "ADR-003: PWA-First for Alpha", "short": "Why the browser is the alpha client"},
            {"path": "architecture/ADR-004-lightning-settlement-backend.md", "title": "ADR-004: Lightning Settlement Backend", "short": "LNbits + phoenixd, custody posture"},
            {"path": "spec/01-engine-architecture.md", "title": "Engine Architecture", "short": "engine architecture & roadmap: ECS, tick loop, planned crate split"},
            {"path": "spec/02-world-format.md", "title": "World Format", "short": "Chunks, compression, persistence"},
            {"path": "spec/03-rendering.md", "title": "Rendering", "short": "wgpu, greedy meshing, GPU tiers"},
            {"path": "spec/04-networking.md", "title": "Networking", "short": "UDP/WebRTC, prediction, anti-DDoS"},
            {"path": "spec/06-bitcoin-integration.md", "title": "Bitcoin Integration", "short": "Design spec — hash-on-mine, reward economics; nothing here is live"},
            {"path": "spec/07-platform-services.md", "title": "Platform Services", "short": "Auth, matchmaking, Agones, moderation"},
            {"path": "spec/08-security-anti-cheat.md", "title": "Security & Anti-Cheat", "short": "Threat model, plugin sandbox, privacy"},
            {"path": "foundations/2026-05-07-engine-commands.md", "title": "Engine Commands (Foundation)", "short": "Chat overlay + parser + dispatcher (plugin-shaped)"},
            {"path": "foundations/2026-09-05-world-chat.md", "title": "World Chat (Foundation)", "short": "Design: tiers, Charter ceiling, guardian copy, room plug — native only"},
            {"path": "superpowers/specs/2026-09-06-online-play-by-contact-design.md", "title": "Online Play by Contact (Design)", "short": "Join a friend's home-hosted world by contact npub — Nostr rendezvous, NAT traversal, admission model"},
            {"path": "roadmap.md", "title": "Roadmap", "short": "Where the project is, what's waiting, what's next"},
        ],
    },
}


def get_section_nav():
    return [
        {"slug": slug, "title": sec["title"], "subtitle": sec.get("subtitle", "")}
        for slug, sec in SECTIONS.items()
    ]


def _common_context():
    return {
        "sections": get_section_nav(),
        "game_url": GAME_URL,
        "marketing_url": MARKETING_URL,
        "project_url": PROJECT_URL,
    }


# --- Routes ---

@app.get("/", response_class=HTMLResponse)
async def home(request: Request):
    return templates.TemplateResponse(
        request=request, name="home.html", context=_common_context()
    )


@app.get("/docs/{section}", response_class=HTMLResponse)
async def section_index(request: Request, section: str):
    if section not in SECTIONS:
        raise HTTPException(status_code=404, detail="Section not found")
    sec = SECTIONS[section]
    docs = []
    for doc in sec["docs"]:
        path = DOCS_ROOT / doc["path"]
        docs.append({**doc, "exists": path.exists()})
    return templates.TemplateResponse(
        request=request, name="section.html",
        context={
            **_common_context(),
            "section_slug": section,
            "section": sec,
            "docs": docs,
        },
    )


@app.get("/docs/{section}/{doc_path:path}", response_class=HTMLResponse)
async def doc_page(request: Request, section: str, doc_path: str):
    if section not in SECTIONS:
        raise HTTPException(status_code=404, detail="Section not found")
    sec = SECTIONS[section]
    doc_entry = None
    for d in sec["docs"]:
        if d["path"] == doc_path:
            doc_entry = d
            break
    if not doc_entry:
        raise HTTPException(status_code=404, detail="Document not found in section")
    path = DOCS_ROOT / doc_path
    if not path.exists():
        raise HTTPException(status_code=404, detail="Document file not found")
    html_content, toc_html = render_md(path)
    return templates.TemplateResponse(
        request=request, name="doc.html",
        context={
            **_common_context(),
            "section_slug": section,
            "section": sec,
            "doc": doc_entry,
            "content": html_content,
            "toc": toc_html,
        },
    )


@app.get("/roadmap", response_class=HTMLResponse)
async def roadmap_page(request: Request):
    path = DOCS_ROOT / "roadmap.md"
    if not path.exists():
        raise HTTPException(status_code=404, detail="Roadmap not found")
    html_content, toc_html = render_md(path)
    return templates.TemplateResponse(
        request=request, name="doc.html",
        context={
            **_common_context(),
            "section_slug": "",
            "section": {"title": "Roadmap"},
            "doc": {"title": "Build Roadmap", "short": "High-level build phases"},
            "content": html_content,
            "toc": toc_html,
        },
    )


def _human_size(size_bytes: int) -> str:
    if size_bytes > 1024 * 1024:
        return f"{size_bytes / (1024 * 1024):.1f} MB"
    return f"{size_bytes / 1024:.0f} KB"


def _sha256_cached(path: Path) -> str:
    """sha256 hex of a file, cached by (path, size, mtime) so the page is cheap to render."""
    st = path.stat()
    key = (str(path), st.st_size, st.st_mtime_ns)
    cached = _sha256_cache.get(key)
    if cached is not None:
        return cached
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    digest = h.hexdigest()
    _sha256_cache[key] = digest
    return digest


def _discover_installers() -> list[dict]:
    """Enumerate the installers present in INSTALLERS_DIR, with size + checksum + per-OS note.

    `publish-installers.yml` no longer `rsync --delete`s this dir (a routine
    linux-only publish must not wipe the other OSes' files), so every past
    release's installer stays on disk. Listing all of them would let a player
    pick a build predating a security fix and would grow the box's disk
    unboundedly (2026-09-27 audit, REVIEW-W6 should-fix #1) — so only the
    newest version per (os, format) is returned; older files stay on disk,
    just unlisted."""
    out: list[dict] = []
    if not INSTALLERS_DIR.is_dir():
        return out
    for f in sorted(INSTALLERS_DIR.iterdir()):
        if not f.is_file():
            continue
        # The -latest- AppImage + its .zsync are the auto-update channel, not a
        # user-facing download — list only the versioned installers.
        if "-latest-" in f.name:
            continue
        meta = _INSTALLER_META.get(f.suffix.lower())
        if meta is None:
            continue
        os_key, fmt, note, severity = meta
        out.append({
            "filename": f.name,
            "os": os_key,
            "os_label": _OS_LABELS[os_key],
            "format": fmt,
            "note": note,
            "severity": severity,
            "size": _human_size(f.stat().st_size),
            "sha256": _sha256_cached(f),
        })
    return filter_newest_per_platform(out)


@app.get("/download", response_class=HTMLResponse)
async def download_page(request: Request):
    binary_path = BUILD_DIR / BINARY_NAME
    binary_available = binary_path.exists()
    binary_size = _human_size(binary_path.stat().st_size) if binary_available else ""
    installers = _discover_installers()
    detected_os = _detect_os(request.headers.get("user-agent", ""))
    return templates.TemplateResponse(
        request=request, name="download.html",
        context={
            **_common_context(),
            "binary_available": binary_available,
            "binary_size": binary_size,
            "installers": installers,
            "detected_os": detected_os,
        },
    )


@app.get("/download/latest.json")
async def latest_json():
    """Machine-readable "what is the newest published build".

    Consumed by the native engine's in-game version indicator
    (`game/engine/src/update_check.rs`) to show
    `AxeNStax v0.2.16 (v0.2.18 available)` in the lobby.

    Built from the SAME `_discover_installers()` call that renders the download
    page, so the advertised version cannot drift from what is downloadable — the
    exact failure that let a June AppImage sit installed for a month without
    anything saying so.

    Also carries `linux_appimage_sha256` + `linux_appimage_url` (added
    2026-09-03) so an in-place AppImage updater can fetch and verify the newest
    build directly, without re-deriving a URL or re-hashing anything itself.
    `linux_appimage_url` points at the versioned `/download/installer/{name}`
    route (the same one the download page links to), not the bare
    `/download/{name}` auto-update-channel route.

    Deliberately plain: no auth, no cookies, no query parameters, nothing logged
    per-caller. It is a static fact about the release, not a telemetry endpoint.
    """
    installers = [
        {"filename": i["filename"], "sha256": i["sha256"]}
        for i in _discover_installers()
    ]
    doc = latest_manifest(
        installers,
        "https://docs.axenstax.org/download",
        "https://docs.axenstax.org/download/installer",
    )
    return JSONResponse(doc, headers={"Cache-Control": "public, max-age=300"})


@app.get("/download/installer/{name}")
async def download_installer(name: str):
    """Serve a built installer from INSTALLERS_DIR (path-traversal guarded)."""
    if not _INSTALLER_NAME_RE.match(name):
        raise HTTPException(status_code=400, detail="Invalid installer name")
    candidate = INSTALLERS_DIR / name
    try:
        resolved = candidate.resolve(strict=True)
    except FileNotFoundError:
        raise HTTPException(status_code=404, detail="Installer not found")
    if INSTALLERS_DIR.resolve() not in resolved.parents:
        raise HTTPException(status_code=400, detail="Path escaped installers root")
    # is_file() guards against a directory that happens to carry an installer suffix
    # (e.g. a `*.dmg/` dir) reaching FileResponse and 500-ing; serve only real files.
    if not resolved.is_file() or resolved.suffix.lower() not in _INSTALLER_META:
        raise HTTPException(status_code=404, detail="Not an installer")
    return FileResponse(
        path=str(resolved),
        filename=name,
        media_type="application/octet-stream",
    )


@app.get("/download/axenstax-engine")
async def download_binary():
    binary_path = BUILD_DIR / BINARY_NAME
    if not binary_path.exists():
        raise HTTPException(status_code=404, detail="Binary not built yet")
    return FileResponse(
        path=str(binary_path),
        filename=BINARY_NAME,
        media_type="application/octet-stream",
    )


@app.get("/download/{name}")
async def download_autoupdate(name: str):
    """Serve the AppImage auto-update channel (the -latest- AppImage + its
    .zsync delta map) at the bare /download/<file> URL embedded in the AppImage.
    Scoped to _AUTOUPDATE_NAMES — anything else 404s. Registered AFTER the
    specific /download/* routes so it never shadows them."""
    if name not in _AUTOUPDATE_NAMES:
        raise HTTPException(status_code=404, detail="Not found")
    candidate = (INSTALLERS_DIR / name).resolve()
    if not candidate.is_file() or INSTALLERS_DIR.resolve() not in candidate.parents:
        raise HTTPException(status_code=404, detail="Not built yet")
    return FileResponse(
        path=str(candidate),
        filename=name,
        media_type="application/octet-stream",
    )


_TEST_SHEET_NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,99}$")
TEST_SHEETS_DIR = DOCS_ROOT / "test-sheets"


def _render_test_sheet(name: str) -> tuple[str, str, str]:
    stem = name[:-3] if name.endswith(".md") else name
    if not _TEST_SHEET_NAME_RE.match(stem):
        raise HTTPException(status_code=400, detail="Invalid test-sheet name")
    candidate = TEST_SHEETS_DIR / f"{stem}.md"
    try:
        resolved = candidate.resolve(strict=True)
    except FileNotFoundError:
        raise HTTPException(status_code=404, detail="Test sheet not found")
    if TEST_SHEETS_DIR.resolve() not in resolved.parents:
        raise HTTPException(status_code=400, detail="Path escaped test-sheets root")
    MD.reset()
    text = resolved.read_text()
    html = MD.convert(text)
    title = stem.replace("-", " ")
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("# ") and not stripped.startswith("## "):
            title = stripped[2:].strip()
            break
    return title, html, stem


@app.get("/print/test-sheet/{name}", response_class=HTMLResponse)
async def print_test_sheet(request: Request, name: str):
    title, html, stem = _render_test_sheet(name)
    return templates.TemplateResponse(
        request=request,
        name="print_test_sheet.html",
        context={
            "title": title,
            "content": html,
            "stem": stem,
        },
    )


@app.get("/help", response_class=HTMLResponse)
async def help_page(request: Request):
    return templates.TemplateResponse(
        request=request, name="help.html",
        context=_common_context(),
    )


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


if __name__ == "__main__":
    import uvicorn
    cert = CERTS_DIR / "cert.pem"
    key = CERTS_DIR / "key.pem"
    ssl_kwargs = {}
    if cert.exists() and key.exists():
        ssl_kwargs = {"ssl_certfile": str(cert), "ssl_keyfile": str(key)}
    uvicorn.run(app, host="0.0.0.0", port=PORT, log_level="info", access_log=False, **ssl_kwargs)
