#!/usr/bin/env python3
"""Axe'n'Stax — marketing landing site (port 8096).

Public-facing product front door. Lives on axenstax.com in production. Routes:
`/` (pitch + Play), `/will-it-run` (WebGPU check), `/roadmap` (product roadmap),
`/safety` (for parents), `/support` ("Support the project" — Lightning tip, merch,
contribute). Points out at the game, learn, wiki, and docs/project. The in-person
merch claim site is deliberately UNLINKED — reachable only by direct code URL.
"""

import logging
import os
from pathlib import Path

from dotenv import load_dotenv
from fastapi import FastAPI, Request
from fastapi.responses import HTMLResponse
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates

load_dotenv()
logging.basicConfig(level=os.environ.get("LOG_LEVEL", "WARNING").upper())
log = logging.getLogger(__name__)

BASE_DIR = Path(__file__).parent
CERTS_DIR = BASE_DIR / "certs"

PORT = int(os.environ.get("PORT", "8096"))
GAME_URL = os.environ.get("GAME_URL", "https://localhost:8094")
DOCS_URL = os.environ.get("DOCS_URL", "https://localhost:8095")
LEARN_URL = os.environ.get("LEARN_URL", "https://localhost:8098")
WIKI_URL = os.environ.get("WIKI_URL", "https://localhost:8097")
PROJECT_URL = os.environ.get("PROJECT_URL", "https://localhost:8099")
# HELD until the source repo is public — blank shows "soon" instead of a 404.
SOURCE_URL = os.environ.get("SOURCE_URL", "").strip()
# Support page: Lightning tip target (LN address / LNURL — blank = "coming soon")
# and the future online shop (blank = "coming soon"). The merch claim site is
# deliberately NOT referenced here — it stays isolated, reachable only by code URL.
LIGHTNING_TIP = os.environ.get("LIGHTNING_TIP_ADDRESS", "").strip()
SHOP_URL = os.environ.get("SHOP_URL", "").strip()
# Patron "get in touch" — a full href (mailto:… / https://… / nostr:…). Patronage
# is a relationship, so this is a contact, not a checkout. Blank = "coming soon".
PATRON_CONTACT = os.environ.get("PATRON_CONTACT", "").strip()

app = FastAPI(title="Axe'n'Stax — Marketing")

# Tight baseline CSP. No external script origins; one stylesheet, no fetches.
_BASELINE_CSP = (
    "default-src 'self'; "
    "script-src 'self'; "
    "style-src 'self' 'unsafe-inline'; "
    "connect-src 'self'; "
    "img-src 'self' data:; "
    "font-src 'self' data:; "
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


app.mount("/static", StaticFiles(directory=str(BASE_DIR / "static")), name="static")
templates = Jinja2Templates(directory=str(BASE_DIR / "templates"))


def _base_context() -> dict:
    return {
        "game_url": GAME_URL, "docs_url": DOCS_URL, "learn_url": LEARN_URL,
        "wiki_url": WIKI_URL, "project_url": PROJECT_URL, "source_url": SOURCE_URL,
    }


def _lightning_qr_data_uri(payload: str):
    """Inline SVG-data-URI QR for the Lightning tip, generated server-side so it
    works under the tight CSP (img-src 'self' data:) with no external scripts.
    Returns None if there's no address or the QR lib isn't installed — the page
    then degrades to the copyable address text only."""
    if not payload:
        return None
    try:
        import segno
    except ImportError:
        return None
    try:
        return segno.make(payload, error="m").svg_data_uri(scale=5, border=2)
    except Exception:  # never let a QR hiccup take down the page
        return None


@app.get("/", response_class=HTMLResponse)
async def home(request: Request):
    return templates.TemplateResponse(request=request, name="home.html", context=_base_context())


@app.get("/will-it-run", response_class=HTMLResponse)
async def will_it_run(request: Request):
    """Self-serve WebGPU compatibility check. No sign-in required.

    Probes navigator.gpu.requestAdapter() in the browser and renders a
    green/red verdict with browser-specific remediation copy. Linked from
    the marketing hero hint so testers can validate their setup before
    bothering with Signet.
    """
    return templates.TemplateResponse(
        request=request, name="will_it_run.html", context=_base_context()
    )


@app.get("/support", response_class=HTMLResponse)
async def support(request: Request):
    """Ways to back the open-source project — Lightning tip, merch, contribute.
    Framed as supporting the project, not a merch store."""
    ctx = _base_context()
    ctx.update({
        "lightning_tip": LIGHTNING_TIP,
        "tip_qr": _lightning_qr_data_uri(LIGHTNING_TIP),
        "shop_url": SHOP_URL,
        "patron_contact": PATRON_CONTACT,
    })
    return templates.TemplateResponse(request=request, name="support.html", context=ctx)


@app.get("/experiences", response_class=HTMLResponse)
async def experiences(request: Request):
    """Experiences — what they are, the two official ones (Hash Dash, Satori Rush),
    and the coming create-and-share. Product-side showcase of the scenario layer."""
    return templates.TemplateResponse(request=request, name="experiences.html", context=_base_context())


@app.get("/roadmap", response_class=HTMLResponse)
async def roadmap(request: Request):
    """Product roadmap — 'what you can look forward to DOING' (experience voice).
    Deliberately shares no sentence with the engineering roadmap on .org; see
    docs/foundations/2026-06-08-site-content-architecture.md §4a."""
    return templates.TemplateResponse(request=request, name="roadmap.html", context=_base_context())


@app.get("/safety", response_class=HTMLResponse)
async def safety(request: Request):
    """For parents — is-this-gambling, real-Bitcoin-and-kids, what sign-in is, and
    privacy. The gatekeeper audience for a kids' game that mentions Bitcoin."""
    return templates.TemplateResponse(request=request, name="safety.html", context=_base_context())


@app.get("/privacy", response_class=HTMLResponse)
async def privacy(request: Request):
    """The one shared /privacy page for all six sites (2026-09-28 audit fix,
    go-live MUST #20) — every other site links here via MARKETING_URL rather
    than carrying its own copy. States exactly and only what the code does
    today. Signed off by the owner 2026-10-04 (DRAFT banner removed)."""
    return templates.TemplateResponse(request=request, name="privacy.html", context=_base_context())


if __name__ == "__main__":
    import uvicorn
    cert = CERTS_DIR / "cert.pem"
    key = CERTS_DIR / "key.pem"
    ssl_kwargs = {}
    if cert.exists() and key.exists():
        ssl_kwargs = {"ssl_certfile": str(cert), "ssl_keyfile": str(key)}
    uvicorn.run(app, host="0.0.0.0", port=PORT, log_level="info", access_log=False, **ssl_kwargs)
