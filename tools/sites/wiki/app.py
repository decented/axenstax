#!/usr/bin/env python3
"""Axe'n'Stax — wiki / player reference (port 8097).

The dense, lookup-driven player reference: "tame a wolf", "smelt iron", "what
does this block do". Lives on wiki.axenstax.com in production. This is the
*reference* half of the player content — the lighter, guided onboarding journey
is the learn site (tools/sites/learn/). Markdown is rendered from docs/player-guide/.
"""

import logging
import os
from pathlib import Path

import markdown
from dotenv import load_dotenv
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import FileResponse, HTMLResponse
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates

load_dotenv()
log = logging.getLogger(__name__)

BASE_DIR = Path(__file__).parent
PROJECT_ROOT = BASE_DIR.parent.parent.parent
DOCS_ROOT = PROJECT_ROOT / "docs"
CERTS_DIR = BASE_DIR / "certs"

PORT = int(os.environ.get("PORT", "8097"))
GAME_URL = os.environ.get("GAME_URL", "https://localhost:8094")
MARKETING_URL = os.environ.get("MARKETING_URL", "https://localhost:8096")

app = FastAPI(title="Axe'n'Stax — Wiki")

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


# --- Content: the player guide (the reference) ---
# Single section, kept in the same SECTIONS shape as the docs/learn sites so the
# shared section.html / doc.html templates render it unchanged.
SECTIONS = {
    "player-guide": {
        "title": "Player Guide",
        "subtitle": "How to play Axe'n'Stax — controls, blocks, mobs, villages, quests, everything",
        "docs": [
            {"path": "player-guide/index.md", "title": "Overview", "short": "Welcome + table of contents"},
            {"path": "player-guide/getting-started.md", "title": "Getting Started", "short": "Your first five minutes"},
            {"path": "player-guide/controls.md", "title": "Controls", "short": "Every key, button, and click — keyboard / mouse / gamepad / touch"},
            {"path": "player-guide/inventory-and-tools.md", "title": "Inventory & Tools", "short": "Hotbar, slots, tool tiers, durability, dropping"},
            {"path": "player-guide/tools-reference.md", "title": "Tools Reference", "short": "Tier ladder, durability, damage, the pickaxe gate"},
            {"path": "player-guide/blocks-and-mining.md", "title": "Blocks & Mining", "short": "Full block reference, mining tiers, ores, Satori"},
            {"path": "player-guide/building-blocks.md", "title": "Building Blocks", "short": "Slabs, stairs, doors, trapdoors, fence gates, walls, glass panes, iron bars, signs, item frames"},
            {"path": "player-guide/storage-blocks.md", "title": "Storage & Workstation Blocks", "short": "Furnace, mill, oven, aging rack, chest, vendor block, plaque"},
            {"path": "player-guide/crafting.md", "title": "Crafting", "short": "The two grids + grid diagrams for every recipe"},
            {"path": "player-guide/smelting.md", "title": "Smelting", "short": "The Furnace turns ore into metal ingots"},
            {"path": "player-guide/build-schematics.md", "title": "Build Schematics", "short": "Capture, develop & rebuild your builds"},
            {"path": "player-guide/workshop.md", "title": "The Workshop", "short": "Blow blocks up and reskin & reshape them — dyes, eyedropper, symmetry, pin, the Wardrobe"},
            {"path": "player-guide/art-galleries.md", "title": "Create an Art Gallery", "short": "An artist's step-by-step: prepare art, upload via the Console, hang it with /exhibit, run a kiosk, share a link"},
            {"path": "player-guide/cinematic-camera.md", "title": "Cinematic Camera (the Director)", "short": "Built-in freecam + film studio (PC) — fly, six modes, hide-HUD, dolly paths, record clips"},
            {"path": "player-guide/electricity.md", "title": "Electricity — Power & Logic", "short": "Cables, switches, lamps, logic gates, generators + light-beam tripwires & motion sensors"},
            {"path": "player-guide/explosives.md", "title": "Explosives — Black Powder & the Blasting Keg", "short": "Black Powder, the Blasting Keg, the Composter, fuse + plunger detonation — demolition, not mining"},
            {"path": "player-guide/combat-and-mobs.md", "title": "Combat & Mobs", "short": "Combat basics and the full 30-mob roster"},
            {"path": "player-guide/wolves.md", "title": "Wolves", "short": "The first tameable companion — tame, follow, sit, fight alongside you"},
            {"path": "player-guide/nostrich.md", "title": "Nostrich", "short": "The purple-ostrich Nostr mascot — eggs, feathers, the Vow"},
            {"path": "player-guide/armour.md", "title": "Armour", "short": "Four slots, five tiers, damage reduction, durability"},
            {"path": "player-guide/cosmetics.md", "title": "Skins & the Wardrobe", "short": "Keep a wardrobe of skins, paint your own in the Workshop (blow up your avatar, paint it like a block), export to wear in real Minecraft, or import from a Minecraft username"},
            {"path": "player-guide/food-and-cooking.md", "title": "Food & Cooking", "short": "Hunger, food values, campfire + Furnace, baking, fire-starting"},
            {"path": "player-guide/farming.md", "title": "Farming", "short": "Grow crops, fibre and dye flowers"},
            {"path": "player-guide/dyes-fibre-and-magnesium.md", "title": "Dyes, Fibre & Magnesium", "short": "Flowers → dyes → décor, cotton/hemp → cloth & canvas, magnesium"},
            {"path": "player-guide/biomes.md", "title": "Biomes", "short": "The launch biomes and how the world picks one"},
            {"path": "player-guide/villages-and-villagers.md", "title": "Villages & Villagers", "short": "Finding villages, professions, dialogue"},
            {"path": "player-guide/quests-and-reputation.md", "title": "Quests & Reputation", "short": "Three quest flavours, reputation tiers, multipliers"},
            {"path": "player-guide/iron-golems.md", "title": "Knights", "short": "The village's defender — how they work + how to fight them"},
            {"path": "player-guide/village-bell.md", "title": "Village Bell", "short": "Found your own village in the wild"},
            {"path": "player-guide/bitcoin-and-sats.md", "title": "Bitcoin & Sats", "short": "What the sat numbers mean — Vendor Block trade, Plaque tipping, plain English"},
            {"path": "player-guide/trade-value.md", "title": "Trade Value & the In-Game Economy", "short": "The complexity ladder, in-game trade scoring, Genesis Block"},
            {"path": "player-guide/game-modes.md", "title": "Game Modes", "short": "Survival vs Creative, difficulty knob"},
            {"path": "player-guide/world-creation-and-spawn.md", "title": "World Creation & Spawn", "short": "Creating worlds, spawn dropdown, save/load"},
            {"path": "player-guide/cloud-and-stash.md", "title": "Cloud Saves & the Stash Column", "short": "Back up worlds encrypted, the Stash traffic-light, browse & adopt from the Stash Column"},
            {"path": "player-guide/maps-and-coords.md", "title": "Maps & Coordinates", "short": "F3 coordinates and finding your way"},
            {"path": "player-guide/chat-and-commands.md", "title": "Chat & Commands", "short": "Chat overlay + /help + /time + /give + /tp"},
            {"path": "player-guide/cheats.md", "title": "Cheats & Commands", "short": "Cheat vs non-cheat commands and the integrity ledger"},
            {"path": "player-guide/split-screen.md", "title": "Split-Screen (Couch Co-op)", "short": "Up to 4 players, controllers + keyboard, per-player state"},
            {"path": "player-guide/multiplayer-lan.md", "title": "Multiplayer (LAN)", "short": "Host and join a game on your local network"},
            {"path": "player-guide/play-with-a-friend-online.md", "title": "Play with a Friend Online", "short": "Host a world at home; a friend in another house joins by invite link"},
            {"path": "player-guide/whats-coming.md", "title": "What's Coming", "short": "What just shipped, what's partial, what's still pending"},
            {"path": "player-guide/minigames.md", "title": "Minigames", "short": "Hash Dash & Satori Rush — the rules in brief"},
            {"path": "player-guide/tips-and-tricks.md", "title": "Tips & Tricks", "short": "Small things that make a big difference"},
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
