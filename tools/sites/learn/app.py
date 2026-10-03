#!/usr/bin/env python3
"""Axe'n'Stax — learn / guided journey (port 8098).

The light, warm, kid- and parent-friendly onboarding journey: you don't consult
it, you're taken through it. Lives on learn.axenstax.com in production. Tonally
this is marketing's friendly cousin, NOT the wiki's sibling — markety, never a
manual. (The dense lookup reference is the wiki site, tools/sites/wiki/.)

Content lives in docs/learn-journey/ — re-voiced from the old docs/tutorials/ into
a warm, read-aloud, phone-glanceable guided journey (the tutorials are left intact).
One markdown source, many surfaces: web page, PDF/print, BYO-AI assistant, and
eventually an in-game egui Guide tab — see
docs/architecture/2026-06-06-domain-and-site-architecture.md (the delivery ladder).
"""

import logging
import os
from pathlib import Path

import markdown
from dotenv import load_dotenv
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import FileResponse, HTMLResponse, RedirectResponse
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates

load_dotenv()
log = logging.getLogger(__name__)

BASE_DIR = Path(__file__).parent
PROJECT_ROOT = BASE_DIR.parent.parent.parent
DOCS_ROOT = PROJECT_ROOT / "docs"
CERTS_DIR = BASE_DIR / "certs"

PORT = int(os.environ.get("PORT", "8098"))
GAME_URL = os.environ.get("GAME_URL", "https://localhost:8094")
MARKETING_URL = os.environ.get("MARKETING_URL", "https://localhost:8096")

app = FastAPI(title="Axe'n'Stax — Learn")

# Tight baseline CSP — pure content site, no scripts, one inline-styled stylesheet.
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

# --- Markdown renderer ---
MD = markdown.Markdown(
    extensions=["tables", "fenced_code", "toc", "attr_list", "pymdownx.tasklist"],
    extension_configs={"pymdownx.tasklist": {"custom_checkbox": True}},
)


def render_md(path: Path) -> tuple[str, str]:
    MD.reset()
    html = MD.convert(path.read_text())
    return html, getattr(MD, "toc", "")


# --- Content: the guided journey ---
# Re-voiced warm/guided content lives in docs/learn-journey/ (the old
# docs/tutorials/ manuals are left untouched). Kept in the SECTIONS shape so the
# shared section.html / doc.html templates render it unchanged.
SECTIONS = {
    "journey": {
        "title": "The Journey",
        "subtitle": "Brand-new? Start here. We'll walk you in, one step at a time.",
        "docs": [
            {"path": "learn-journey/index.md", "title": "Welcome", "short": "What's ahead — the whole path, start to finish"},
            {"path": "learn-journey/getting-around.md", "title": "Getting Around", "short": "Move, look, hold things, open your bag"},
            {"path": "learn-journey/first-blocks.md", "title": "Your First Blocks", "short": "Break one, carry it, place it back down"},
            {"path": "learn-journey/first-workbench.md", "title": "Make a Crafting Table", "short": "Turn wood into your crafting table"},
            {"path": "learn-journey/first-tools.md", "title": "Make Your First Tools", "short": "A wooden axe and pickaxe, then stone"},
            {"path": "learn-journey/cook-on-a-campfire.md", "title": "Cook on a Campfire", "short": "Build a fire, cook some food, eat"},
            {"path": "learn-journey/surviving-your-first-night.md", "title": "Surviving Your First Night", "short": "Light, a shelter, and a bed before dark"},
            {"path": "learn-journey/keep-your-stuff-in-a-chest.md", "title": "Keep Your Stuff in a Chest", "short": "A safe box for everything you're not carrying"},
            {"path": "learn-journey/your-first-farm.md", "title": "Start Your First Farm", "short": "Plant seeds, grow food, harvest it"},
            {"path": "learn-journey/smelt-iron-in-a-furnace.md", "title": "Smelt Iron in a Furnace", "short": "Melt ore into metal and make iron tools"},
            {"path": "learn-journey/stand-your-ground.md", "title": "Stand Your Ground", "short": "Make a sword and win a fight"},
            {"path": "learn-journey/tame-a-wolf.md", "title": "Tame a Wolf", "short": "Win over a companion who fights on your side"},
            {"path": "learn-journey/what-the-hash-means.md", "title": "What the Hash Means", "short": "The real Bitcoin maths under every swing"},
            {"path": "learn-journey/capture-a-schematic.md", "title": "Save Your First Build", "short": "Capture a build and rebuild it anywhere"},
            {"path": "learn-journey/build-with-shapes.md", "title": "Build with Shapes", "short": "Slabs, stairs, and a door that opens — turn a box into a building"},
            {"path": "learn-journey/make-it-your-own.md", "title": "Make It Your Own", "short": "Blow up a block, paint it, and reskin your world"},
            {"path": "learn-journey/switch-on-a-lamp.md", "title": "Switch On a Lamp", "short": "Wire up your first circuit — a lamp on a switch"},
            {"path": "learn-journey/light-a-blasting-keg.md", "title": "Light a Blasting Keg", "short": "Craft a keg, light the fuse, and blow a crater — by hand and by plunger"},
            {"path": "learn-journey/win-hash-dash.md", "title": "How to Win Hash Dash", "short": "The 3-minute mining sprint"},
            {"path": "learn-journey/win-satori-rush.md", "title": "How to Win Satori Rush", "short": "From nothing to your first Satori"},
            {"path": "learn-journey/film-a-cinematic-shot.md", "title": "Film Your World", "short": "Fly the built-in camera, circle your build, and make a gliding flythrough (free desktop app)"},
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
    docs = [{**doc, "exists": (DOCS_ROOT / doc["path"]).exists()} for doc in sec["docs"]]
    return templates.TemplateResponse(
        request=request, name="section.html",
        context={**_common_context(), "section_slug": section, "section": sec, "docs": docs},
    )


@app.get("/docs/journey/learn-journey/the-hash-and-your-first-sats.md")
async def _redirect_old_hash_lesson_slug():
    """2026-09-28 audit fix (SHOULD): the URL said 'first-sats' though the
    lesson never earns any — renamed to match the title, redirect kept for
    anyone with the old link bookmarked."""
    return RedirectResponse(url="/docs/journey/learn-journey/what-the-hash-means.md", status_code=301)


@app.get("/docs/{section}/{doc_path:path}", response_class=HTMLResponse)
async def doc_page(request: Request, section: str, doc_path: str):
    if section not in SECTIONS:
        raise HTTPException(status_code=404, detail="Section not found")
    sec = SECTIONS[section]
    doc_entry = next((d for d in sec["docs"] if d["path"] == doc_path), None)
    if not doc_entry:
        raise HTTPException(status_code=404, detail="Document not found in section")
    path = DOCS_ROOT / doc_path
    if not path.exists():
        raise HTTPException(status_code=404, detail="Document file not found")
    html_content, toc_html = render_md(path)
    return templates.TemplateResponse(
        request=request, name="doc.html",
        context={
            **_common_context(), "section_slug": section, "section": sec,
            "doc": doc_entry, "content": html_content, "toc": toc_html,
        },
    )


@app.get("/help/cert.pem")
async def download_cert():
    cert_path = CERTS_DIR / "cert.pem"
    if not cert_path.exists():
        raise HTTPException(status_code=404, detail="Certificate not generated yet")
    return FileResponse(
        path=str(cert_path), filename="axenstax-cert.pem", media_type="application/x-pem-file"
    )


if __name__ == "__main__":
    import uvicorn
    cert = CERTS_DIR / "cert.pem"
    key = CERTS_DIR / "key.pem"
    ssl_kwargs = {}
    if cert.exists() and key.exists():
        ssl_kwargs = {"ssl_certfile": str(cert), "ssl_keyfile": str(key)}
    uvicorn.run(app, host="0.0.0.0", port=PORT, log_level="info", access_log=False, **ssl_kwargs)
