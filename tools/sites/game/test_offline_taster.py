"""Regression tests for T0-4 (2026-10-05): the web taster is fully OFFLINE.

The game page may talk only to its own origin. That is enforced three ways, and
this file pins the first two (the third, tools/smoke/forbidden-symbol.mjs, scans
the built bundle in check.sh --smoke):

  1. the CSP header says `connect-src 'self'` (no `wss:`, no hosts) and keeps
     `'wasm-unsafe-eval'` in script-src so the WASM engine can still instantiate;
  2. the served page carries no relay / Stash / Beacon / identity code, no
     `relay.trotters.cc`, and loads only same-origin scripts that exist.

The page is served from game/engine/dist/index.html, which trunk rebuilds, so the
served-page tests point the app at a temp dist built from the SOURCE page
(game/engine/index.html). That keeps them independent of whether a trunk build
happens to be present or stale.

Run: cd tools/sites/game && .venv/bin/python -m pytest test_offline_taster.py -q
"""

import os
import re
import sys
from pathlib import Path

import pytest
from starlette.testclient import TestClient

sys.path.insert(0, os.path.dirname(__file__))

import app as appmod  # noqa: E402

GAME_DIR = Path(__file__).parent
SOURCE_PAGE = GAME_DIR.parent.parent.parent / "game" / "engine" / "index.html"
STATIC = GAME_DIR / "static"

# Strings that must never be on the served page or in its runtime JS.
# (Mirrors OFFLINE_MARKERS in tools/smoke/forbidden-symbol.mjs.)
FORBIDDEN = [
    "relay.trotters.cc",
    "trotters",
    "axenstax-relay",
    "AxeStash",
    "AxeRelay",
    "AxeBeacon",
    "AxeCloud",
    "AxeNostrNip46",
    "AxeHandle",
    "__axenstax_get_signer",
    "stash.iife",
    "relay.iife",
    "beacon.iife",
    "nostr-tools-nip46",
    "blossom-url",
    "cloud-save",
]

# Retired files that must not be served any more.
GONE = [
    "/static/vendor/stash.iife.js",
    "/static/vendor/relay.iife.js",
    "/static/vendor/beacon.iife.js",
    "/static/vendor/nostr-tools-nip46.iife.js",
    "/static/beacon.js",
    "/static/relay-query.js",
    "/static/persona-handle.js",
    "/static/noble-curves.js",
]


@pytest.fixture
def client(tmp_path, monkeypatch):
    dist = tmp_path / "dist"
    dist.mkdir()
    (dist / "index.html").write_text(SOURCE_PAGE.read_text())
    monkeypatch.setattr(appmod, "WASM_DIST", dist)
    return TestClient(appmod.app, raise_server_exceptions=True)


def _directives(csp: str) -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}
    for part in csp.split(";"):
        part = part.strip()
        if part:
            name, *vals = part.split()
            out[name] = vals
    return out


def test_game_page_csp_connect_src_is_self_only(client):
    r = client.get("/")
    assert r.status_code == 200
    d = _directives(r.headers["content-security-policy"])
    assert d["connect-src"] == ["'self'"], d["connect-src"]


def test_game_page_csp_keeps_wasm_and_tight_directives(client):
    d = _directives(client.get("/").headers["content-security-policy"])
    assert "'wasm-unsafe-eval'" in d["script-src"]
    assert "'unsafe-eval'" not in d["script-src"]
    assert d["default-src"] == ["'self'"]
    assert d["frame-ancestors"] == ["'none'"]
    assert d["object-src"] == ["'none'"]
    assert d["form-action"] == ["'self'"]
    # no directive may open the page to another host or any websocket
    for name, vals in d.items():
        for v in vals:
            assert v not in ("*", "wss:", "ws:", "https:", "http:"), f"{name} allows {v}"
            assert not re.match(r"^(wss?|https?)://", v), f"{name} names a remote host: {v}"


def test_baseline_csp_is_also_connect_self_only():
    d = _directives(appmod._BASELINE_CSP)
    assert d["connect-src"] == ["'self'"]


@pytest.mark.parametrize("needle", FORBIDDEN)
def test_served_page_has_no_relay_stash_beacon_or_identity_code(client, needle):
    text = client.get("/").text
    assert needle.lower() not in text.lower(), f"served page still mentions {needle!r}"


def test_served_page_loads_only_same_origin_scripts_that_exist(client):
    text = client.get("/").text
    srcs = re.findall(r"<script[^>]*\ssrc=[\"']([^\"']+)[\"']", text, re.IGNORECASE)
    assert srcs, "expected the page to load scripts"
    for src in srcs:
        assert src.startswith("/") and not src.startswith("//"), f"cross-origin script: {src}"
        assert client.get(src).status_code == 200, f"{src} is referenced but not served"


def test_runtime_static_js_makes_no_cross_origin_calls(client):
    text = client.get("/").text
    srcs = re.findall(r"<script[^>]*\ssrc=[\"'](/static/[^\"']+)[\"']", text, re.IGNORECASE)
    assert srcs
    for src in srcs:
        js = (GAME_DIR / src.lstrip("/")).read_text()
        for needle in FORBIDDEN:
            assert needle not in js, f"{src} mentions {needle!r}"
        assert "new WebSocket(" not in js, f"{src} opens a WebSocket"
        assert not re.search(r"fetch\(\s*[`'\"]https?:", js), f"{src} fetches a remote URL"


@pytest.mark.parametrize("path", GONE)
def test_retired_files_are_not_served(client, path):
    assert client.get(path).status_code == 404


def test_retired_files_are_gone_from_disk():
    assert not (STATIC / "vendor").exists()
    for p in GONE:
        assert not (GAME_DIR / p.lstrip("/")).exists(), p


def test_engine_bridge_globals_the_wasm_still_calls_are_defined():
    # The wasm imports these (wasm_save.rs / open_stash.rs). Several are called
    # without a presence guard, so they must exist as stubs even though cloud
    # save and open-stash sharing are not available on web.
    js = (STATIC / "cloud.js").read_text()
    for sym in (
        "axenstax_cosmetic_save", "axenstax_cosmetic_load", "axenstax_cosmetic_reset",
        "axenstax_skinwardrobe_save", "axenstax_skinwardrobe_load",
        "axenstax_wardrobe_save", "axenstax_wardrobe_load", "axenstax_wardrobe_reset",
        "axenstax_wardrobe_remember_get", "axenstax_wardrobe_remember_set",
        "axenstax_ghost_download", "axenstax_skin_download", "axenstax_mc_skin_fetch",
        "axenstax_cloud_available", "axenstax_cloud_list", "axenstax_cloud_restore",
        "axenstax_sync_stash_status",
        "axenstax_openstash_available", "axenstax_openstash_publish",
        "axenstax_openstash_list", "axenstax_openstash_download",
        "axenstax_openstash_follow", "axenstax_openstash_unfollow",
        "axenstax_openstash_following",
    ):
        assert f"window.{sym} =" in js, f"cloud.js must still define window.{sym}"
    # the stubs must report "unavailable"
    assert "window.axenstax_cloud_available = () => false" in js
    assert "window.axenstax_openstash_available = () => false" in js


def test_mc_skin_import_stays_same_origin():
    # The only skin lookup the page makes is our own /mc-skin proxy (Mojang is
    # reached server-side), so connect-src 'self' keeps it working.
    js = (STATIC / "cloud.js").read_text()
    assert "fetch('/mc-skin?'" in js


def test_service_worker_cache_name_was_bumped(client):
    sw = client.get("/sw.js").text
    m = re.search(r"const CACHE_REV = (\d+);", sw)
    assert m and int(m.group(1)) >= 2, "bump CACHE_REV when /static changes without a wasm change"
    assert "'axenstax-r' + CACHE_REV" in sw


def test_no_worlds_proxy_routes(client):
    # The old server-side cloud-save proxy + manifest endpoints are gone.
    assert client.post("/worlds/upload", content=b"x").status_code >= 400
    assert client.get("/worlds/download/" + "ab" * 32).status_code >= 400
    assert client.get("/worlds/manifest").status_code >= 400
