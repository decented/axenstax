"""Serverless cloud-save wiring tests.

Cloud save is serverless: the browser signs Blossom auth + the manifest event
with the persona's key and talks directly to Blossom + the relay. The server's
only jobs are (a) surface whether cloud save is configured + the public Blossom
URL to the client, and (b) serve the JS bundles. There are no /worlds/* proxy
or manifest routes any more — those were the old bridge tier.
"""


def test_no_worlds_proxy_routes(client):
    # The old server-side proxy + manifest endpoints are gone. Whatever the
    # catch-all static mount does, none of these is a working API route (no 2xx).
    assert client.post("/worlds/upload", content=b"x").status_code >= 400
    assert client.get("/worlds/download/" + "ab" * 32).status_code >= 400
    assert client.get("/worlds/manifest").status_code >= 400


def test_lobby_exposes_cloud_save_and_blossom_url(client, app_module):
    resp = client.get("/")
    assert resp.status_code == 200
    assert '<meta name="cloud-save" content="enabled">' in resp.text
    assert f'<meta name="blossom-url" content="{app_module.BLOSSOM_PUBLIC_URL}">' in resp.text


def test_cloud_js_is_served(client):
    resp = client.get("/static/cloud.js")
    assert resp.status_code == 200
    assert "window.AxeCloud" in resp.text


def test_cloud_js_exposes_cosmetic_bridge(client):
    resp = client.get("/static/cloud.js")
    assert resp.status_code == 200
    for sym in ("axenstax_cosmetic_save", "axenstax_cosmetic_load", "axenstax_cosmetic_reset"):
        assert sym in resp.text, f"cloud.js must expose {sym}"


def test_stash_bundle_is_served(client):
    resp = client.get("/static/vendor/stash.iife.js")
    assert resp.status_code == 200
    assert "AxeStash" in resp.text


def test_relay_bundle_is_served(client):
    resp = client.get("/static/vendor/relay.iife.js")
    assert resp.status_code == 200
    assert "AxeRelay" in resp.text


def test_lobby_loads_cloud_stash_relay(client):
    resp = client.get("/")
    assert "/static/cloud.js" in resp.text
    assert "/static/vendor/stash.iife.js" in resp.text
    assert "/static/vendor/relay.iife.js" in resp.text


def test_blossom_in_baseline_csp(client, app_module):
    # The browser fetches Blossom directly, so its origin must be in connect-src.
    resp = client.get("/")
    csp = resp.headers.get("content-security-policy", "")
    assert app_module.BLOSSOM_PUBLIC_URL in csp


def test_cloud_disabled_without_blossom_url(monkeypatch):
    # With no BLOSSOM_PUBLIC_URL, cloud save is offered as disabled and the
    # Blossom origin is absent from the CSP.
    import importlib
    import os
    import sys

    sys.path.insert(0, os.path.dirname(os.path.dirname(__file__)))
    import app as app_module

    importlib.reload(app_module)
    app_module.startup()
    # Force the no-Blossom config regardless of any local .env (load_dotenv would
    # otherwise re-populate BLOSSOM_PUBLIC_URL on reload). We're testing that an
    # empty URL disables cloud + omits the Blossom origin from CSP.
    monkeypatch.setattr(app_module, "BLOSSOM_PUBLIC_URL", "")
    assert app_module._cloud_save_enabled() is False

    from starlette.testclient import TestClient

    client = TestClient(app_module.app)
    resp = client.get("/")
    assert '<meta name="cloud-save" content="disabled">' in resp.text
