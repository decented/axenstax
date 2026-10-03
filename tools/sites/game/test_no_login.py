"""Regression tests for the 2026-09-27 audit fix: the game site (web taster)
must carry NO Signet login surface at all — no /auth/* routes, no cookie ever
set. The web build is an anonymous, login-free local sandbox (rewritten
2026-06-27); this pins that down so it can't silently regrow.

Run: cd tools/sites/game && .venv/bin/python -m pytest test_no_login.py -q
"""

import os
import sys

import pytest
from starlette.testclient import TestClient

sys.path.insert(0, os.path.dirname(__file__))

import app as appmod  # noqa: E402

client = TestClient(appmod.app, raise_server_exceptions=True)


def test_auth_verify_is_gone():
    r = client.post("/auth/verify", json={}, headers={"X-Requested-With": "fetch"})
    assert r.status_code == 404


@pytest.mark.parametrize("path", ["/auth/challenge", "/auth/whoami", "/auth/logout"])
def test_every_auth_route_is_gone(path):
    r = client.get(path)
    assert r.status_code == 404, f"{path} should 404 (no auth router mounted)"


def test_api_feedback_voice_proxy_is_gone():
    r = client.post("/api/feedback")
    assert r.status_code == 404


@pytest.mark.parametrize("path", ["/", "/game"])
def test_no_set_cookie_header_from_anywhere(path):
    r = client.get(path, follow_redirects=False)
    assert "set-cookie" not in {k.lower() for k in r.headers.keys()}


def test_no_signed_cookie_module_left_to_import():
    # Belt-and-suspenders: auth.py (the router + HMAC cookie machinery) was
    # deleted with this fix, not just unmounted.
    import importlib.util

    game_dir = os.path.dirname(__file__)
    assert importlib.util.find_spec is not None
    assert not os.path.exists(os.path.join(game_dir, "auth.py"))
    assert not os.path.exists(os.path.join(game_dir, "nostr_auth.py"))
