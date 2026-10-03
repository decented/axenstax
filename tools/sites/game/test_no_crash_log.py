"""Regression test for the 2026-10-03 privacy cut: the web taster stores no
crash text server-side. /api/wasm-error and its wasm-errors.log are gone, and
no page script posts to it any more.

Run: cd tools/sites/game && .venv/bin/python -m pytest test_no_crash_log.py -q
"""

import os
import sys
from pathlib import Path

from starlette.testclient import TestClient

sys.path.insert(0, os.path.dirname(__file__))

import app as appmod  # noqa: E402

client = TestClient(appmod.app, raise_server_exceptions=True)


def test_wasm_error_route_is_gone():
    r = client.post("/api/wasm-error", content=b"boom")
    assert r.status_code in (404, 405)


def test_no_static_script_posts_crash_reports():
    static = Path(__file__).parent / "static"
    for js in static.rglob("*.js"):
        assert "/api/wasm-error" not in js.read_text(errors="replace"), js
