#!/usr/bin/env python3
"""Axe'n'Stax — open-source project home (port 8099).

The builder front door: what the engine is, why it's a full custom build, how to
run your own server, and where the specs live. Lives on axenstax.org in production
(the .org = open-source project, .com = the product). Single page.

NOTE: the source repo is PRIVATE for now (opening soon). The "Source" / contribute
surface is therefore HELD — the page shows "source opening soon" instead of a
public GitHub link that would 404 for outsiders. Flip SOURCE_URL on (env) the day
the repo goes public; that's the only change needed here.
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
log = logging.getLogger(__name__)

BASE_DIR = Path(__file__).parent
CERTS_DIR = BASE_DIR / "certs"

PORT = int(os.environ.get("PORT", "8099"))
GAME_URL = os.environ.get("GAME_URL", "https://localhost:8094")
DOCS_URL = os.environ.get("DOCS_URL", "https://localhost:8095")
LEARN_URL = os.environ.get("LEARN_URL", "https://localhost:8098")
WIKI_URL = os.environ.get("WIKI_URL", "https://localhost:8097")
MARKETING_URL = os.environ.get("MARKETING_URL", "https://localhost:8096")
# HELD until the repo is public. Blank => the template shows "source opening soon"
# rather than a link that 404s. Set to https://github.com/decented/axenstax on open.
SOURCE_URL = os.environ.get("SOURCE_URL", "").strip()

app = FastAPI(title="Axe'n'Stax — Project")

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


@app.get("/", response_class=HTMLResponse)
async def home(request: Request):
    return templates.TemplateResponse(
        request=request, name="home.html",
        context={
            "game_url": GAME_URL,
            "docs_url": DOCS_URL,
            "learn_url": LEARN_URL,
            "wiki_url": WIKI_URL,
            "marketing_url": MARKETING_URL,
            "source_url": SOURCE_URL,
        },
    )


def _ctx() -> dict:
    return {
        "game_url": GAME_URL, "docs_url": DOCS_URL, "learn_url": LEARN_URL,
        "wiki_url": WIKI_URL, "marketing_url": MARKETING_URL, "source_url": SOURCE_URL,
    }


@app.get("/built", response_class=HTMLResponse)
async def built(request: Request):
    """The before-now track record — what's already shipped, the real numbers, and
    the from-scratch primitive stack. The funding-critical 'momentum, not a wish-list'
    page. See docs/foundations/2026-06-08-site-content-architecture.md §4."""
    return templates.TemplateResponse(request=request, name="built.html", context=_ctx())


@app.get("/roadmap", response_class=HTMLResponse)
async def roadmap(request: Request):
    """Engineering roadmap — what's becoming reusable/open, in 'use & contribute'
    voice (vs the product roadmap on .com). Phased detail lives on docs/roadmap."""
    return templates.TemplateResponse(request=request, name="roadmap.html", context=_ctx())


if __name__ == "__main__":
    import uvicorn
    cert = CERTS_DIR / "cert.pem"
    key = CERTS_DIR / "key.pem"
    ssl_kwargs = {}
    if cert.exists() and key.exists():
        ssl_kwargs = {"ssl_certfile": str(cert), "ssl_keyfile": str(key)}
    uvicorn.run(app, host="0.0.0.0", port=PORT, log_level="info", access_log=False, **ssl_kwargs)
