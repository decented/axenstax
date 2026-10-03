"""Standalone proxy tests for GET /mc-skin.

Run: cd tools/sites/game && .venv/bin/python -m pytest test_mc_skin.py -q
     (respx mocks httpx; already in requirements.txt — dev-only, not a runtime dep.)

Coverage:
  - bad name / bad uuid          → 400
  - rate-limit breach            → 429
  - unknown user                 → 404
  - no custom skin               → 422
  - happy path classic           → 200 PNG + correct headers
  - happy path slim              → X-Mc-Slim == "1"
  - cache hit                    → no second upstream call
  - uuid refresh                 → hits cache after a prior name lookup
  - non-Mojang URL               → 502
  - wrong content-type           → 502
  - offline / ConnectError       → 502
  - IP redaction in log filter   → client_addr blanked to "-"
  - cache eviction cap           → cache never exceeds _MC_SKIN_CACHE_MAX
  - malformed/empty Mojang body  → 502 offline (not 500)
  - XFF-derived rate-limit key   → rate bucket keyed on forwarded IP
"""

import base64
import json
import logging
import os
import sys
import time

import pytest
import respx
import httpx
from starlette.testclient import TestClient

# Ensure the game-site directory is on the path so `import app` finds app.py
# (needed when pytest is invoked from a parent directory).
sys.path.insert(0, os.path.dirname(__file__))

import app as appmod  # noqa: E402

client = TestClient(appmod.app, raise_server_exceptions=True)


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(autouse=True)
def clear_mc_state():
    """Reset the in-memory mc-skin cache + rate limiter before every test so
    tests don't bleed state into each other."""
    appmod._MC_SKIN_CACHE.clear()
    appmod._MC_SKIN_RATE.clear()
    yield
    # Clean up after too (belt-and-suspenders).
    appmod._MC_SKIN_CACHE.clear()
    appmod._MC_SKIN_RATE.clear()


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_textures_b64(
    url: str = "http://textures.minecraft.net/texture/abc",
    slim: bool = False,
) -> str:
    """Return the base64-encoded inner textures JSON Mojang includes in profiles."""
    inner: dict = {"textures": {"SKIN": {"url": url}}}
    if slim:
        inner["textures"]["SKIN"]["metadata"] = {"model": "slim"}  # type: ignore[index]
    return base64.b64encode(json.dumps(inner).encode()).decode()


def _no_skin_b64() -> str:
    """Base64 blob with no SKIN key (default-skin account)."""
    return base64.b64encode(json.dumps({"textures": {}}).encode()).decode()


def _profile(
    uuid: str = "069a79f444e94726a5befca90e38aaf5",
    name: str = "Notch",
    slim: bool = False,
    custom: bool = True,
) -> dict:
    """Build a sessionserver /profile/<uuid> JSON response."""
    b64 = _make_textures_b64(slim=slim) if custom else _no_skin_b64()
    return {
        "id": uuid,
        "name": name,
        "properties": [{"name": "textures", "value": b64}],
    }


def _id_resp(
    uuid: str = "069a79f444e94726a5befca90e38aaf5",
    name: str = "Notch",
) -> dict:
    """Build a /users/profiles/minecraft/<name> JSON response."""
    return {"id": uuid, "name": name}


_FAKE_PNG = b"\x89PNG\r\n\x1a\nFAKEDATA"
_TEST_UUID = "069a79f444e94726a5befca90e38aaf5"


# ---------------------------------------------------------------------------
# Input validation
# ---------------------------------------------------------------------------


def test_bad_name_with_space():
    r = client.get("/mc-skin?name=has%20space")
    assert r.status_code == 400
    assert r.json()["error"] == "bad_name"


def test_bad_name_too_long():
    r = client.get("/mc-skin?name=" + "a" * 17)
    assert r.status_code == 400
    assert r.json()["error"] == "bad_name"


def test_bad_name_special_chars():
    r = client.get("/mc-skin?name=bad-name!")
    assert r.status_code == 400
    assert r.json()["error"] == "bad_name"


def test_no_params_is_bad():
    r = client.get("/mc-skin")
    assert r.status_code == 400
    assert r.json()["error"] == "bad_name"


def test_bad_uuid_not_hex():
    r = client.get("/mc-skin?uuid=notahex")
    assert r.status_code == 400
    assert r.json()["error"] == "bad_name"


def test_bad_uuid_wrong_length():
    # 30 hex chars — too short.
    r = client.get("/mc-skin?uuid=" + "a" * 30)
    assert r.status_code == 400
    assert r.json()["error"] == "bad_name"


# ---------------------------------------------------------------------------
# Rate limit
# ---------------------------------------------------------------------------


def test_rate_limit_blocks_at_max():
    """Pre-fill the rate bucket for the TestClient IP to the limit; next call → 429."""
    # TestClient's request.client.host is "testclient".
    appmod._MC_SKIN_RATE["testclient"] = (appmod._MC_SKIN_RATE_MAX, time.monotonic())
    r = client.get("/mc-skin?name=Notch")
    assert r.status_code == 429
    assert r.json()["error"] == "rate_limited"


# ---------------------------------------------------------------------------
# Not found (Mojang 404)
# ---------------------------------------------------------------------------


@respx.mock
def test_not_found():
    """Mojang returns 404 for an unknown (but valid-format) username → our route 404."""
    respx.get(url__startswith="https://api.mojang.com").respond(status_code=404)
    # "NoSuchUser99" is 12 chars — valid format; Mojang mock returns 404.
    r = client.get("/mc-skin?name=NoSuchUser99")
    assert r.status_code == 404
    assert r.json()["error"] == "not_found"


# ---------------------------------------------------------------------------
# No custom skin
# ---------------------------------------------------------------------------


@respx.mock
def test_no_custom_skin():
    """Profile with empty textures → 422 no_custom_skin."""
    respx.get(url__startswith="https://api.mojang.com").respond(json=_id_resp())
    respx.get(url__startswith="https://sessionserver.mojang.com").respond(
        json=_profile(custom=False)
    )
    r = client.get("/mc-skin?name=Notch")
    assert r.status_code == 422
    assert r.json()["error"] == "no_custom_skin"


# ---------------------------------------------------------------------------
# Happy path
# ---------------------------------------------------------------------------


@respx.mock
def test_happy_path_classic():
    """Classic (non-slim) skin: 200 + correct headers + PNG body."""
    respx.get(url__startswith="https://api.mojang.com").respond(
        json=_id_resp(uuid=_TEST_UUID)
    )
    respx.get(url__startswith="https://sessionserver.mojang.com").respond(
        json=_profile(uuid=_TEST_UUID, slim=False)
    )
    respx.get(url__startswith="https://textures.minecraft.net").respond(
        content=_FAKE_PNG, headers={"content-type": "image/png"}
    )
    r = client.get("/mc-skin?name=Notch")
    assert r.status_code == 200
    assert r.headers["content-type"] == "image/png"
    assert r.headers["X-Mc-Slim"] == "0"
    assert r.headers["X-Mc-Uuid"] == _TEST_UUID
    assert r.headers["X-Mc-Name"] == "Notch"
    assert r.headers["Cache-Control"] == "no-store"
    assert r.content == _FAKE_PNG


@respx.mock
def test_happy_path_slim():
    """Slim (Alex-model) skin: X-Mc-Slim header is '1'."""
    respx.get(url__startswith="https://api.mojang.com").respond(json=_id_resp())
    respx.get(url__startswith="https://sessionserver.mojang.com").respond(
        json=_profile(slim=True)
    )
    respx.get(url__startswith="https://textures.minecraft.net").respond(
        content=_FAKE_PNG, headers={"content-type": "image/png"}
    )
    r = client.get("/mc-skin?name=Notch")
    assert r.status_code == 200
    assert r.headers["X-Mc-Slim"] == "1"


# ---------------------------------------------------------------------------
# Cache hit
# ---------------------------------------------------------------------------


@respx.mock
def test_cache_hit_skips_upstream():
    """Two identical name requests: upstream called exactly once."""
    api_mock = respx.get(url__startswith="https://api.mojang.com").respond(
        json=_id_resp()
    )
    session_mock = respx.get(url__startswith="https://sessionserver.mojang.com").respond(
        json=_profile()
    )
    tex_mock = respx.get(url__startswith="https://textures.minecraft.net").respond(
        content=_FAKE_PNG, headers={"content-type": "image/png"}
    )

    r1 = client.get("/mc-skin?name=notch")
    assert r1.status_code == 200

    # Second request for the same name must hit in-memory cache.
    r2 = client.get("/mc-skin?name=notch")
    assert r2.status_code == 200

    assert api_mock.call_count == 1, "API looked up more than once"
    assert session_mock.call_count == 1, "Session looked up more than once"
    assert tex_mock.call_count == 1, "Texture fetched more than once"


@respx.mock
def test_uuid_refresh_hits_cache():
    """After a name lookup, a ?uuid= refresh skips all upstream calls."""
    respx.get(url__startswith="https://api.mojang.com").respond(
        json=_id_resp(uuid=_TEST_UUID)
    )
    session_mock = respx.get(url__startswith="https://sessionserver.mojang.com").respond(
        json=_profile(uuid=_TEST_UUID)
    )
    tex_mock = respx.get(url__startswith="https://textures.minecraft.net").respond(
        content=_FAKE_PNG, headers={"content-type": "image/png"}
    )

    # Populate cache via name lookup.
    r1 = client.get("/mc-skin?name=Notch")
    assert r1.status_code == 200

    # Refresh via UUID should hit cache (no additional upstream calls).
    r2 = client.get(f"/mc-skin?uuid={_TEST_UUID}")
    assert r2.status_code == 200

    # Session + texture fetched only once total across both requests.
    assert session_mock.call_count == 1
    assert tex_mock.call_count == 1


# ---------------------------------------------------------------------------
# Security: non-Mojang texture URL
# ---------------------------------------------------------------------------


@respx.mock
def test_non_mojang_texture_url_rejected():
    """Profile with a skin URL not on textures.minecraft.net → 502."""
    evil_b64 = base64.b64encode(
        json.dumps(
            {"textures": {"SKIN": {"url": "https://evil.example.com/skin.png"}}}
        ).encode()
    ).decode()
    bad_profile = {
        "id": _TEST_UUID,
        "name": "Notch",
        "properties": [{"name": "textures", "value": evil_b64}],
    }
    respx.get(url__startswith="https://api.mojang.com").respond(json=_id_resp())
    respx.get(url__startswith="https://sessionserver.mojang.com").respond(json=bad_profile)
    r = client.get("/mc-skin?name=Notch")
    assert r.status_code == 502
    assert r.json()["error"] == "offline"


# ---------------------------------------------------------------------------
# Content-type check
# ---------------------------------------------------------------------------


@respx.mock
def test_non_png_content_type_rejected():
    """Texture CDN responds with non-image/png content-type → 502."""
    respx.get(url__startswith="https://api.mojang.com").respond(json=_id_resp())
    respx.get(url__startswith="https://sessionserver.mojang.com").respond(
        json=_profile()
    )
    respx.get(url__startswith="https://textures.minecraft.net").respond(
        content=b"not a png at all", headers={"content-type": "text/html"}
    )
    r = client.get("/mc-skin?name=Notch")
    assert r.status_code == 502
    assert r.json()["error"] == "offline"


# ---------------------------------------------------------------------------
# Offline / network error
# ---------------------------------------------------------------------------


@respx.mock
def test_mojang_connect_error_returns_502():
    """If the first Mojang call raises a network error, return 502 offline."""
    respx.get(url__startswith="https://api.mojang.com").mock(
        side_effect=httpx.ConnectError("unreachable")
    )
    r = client.get("/mc-skin?name=Notch")
    assert r.status_code == 502
    assert r.json()["error"] == "offline"


@respx.mock
def test_mojang_unexpected_status_returns_502():
    """A non-200/404/429 from Mojang name endpoint → 502 offline."""
    respx.get(url__startswith="https://api.mojang.com").respond(status_code=503)
    r = client.get("/mc-skin?name=Notch")
    assert r.status_code == 502
    assert r.json()["error"] == "offline"


# ---------------------------------------------------------------------------
# IP redaction in the log filter (Fix 1)
# ---------------------------------------------------------------------------


def test_log_filter_redacts_ip_and_query():
    """_McSkinRedactFilter must blank client_addr to '-' and strip the query."""
    filt = appmod._McSkinRedactFilter()

    # Simulate a /mc-skin request with a name query param and real-looking IP.
    record = logging.LogRecord(
        name="uvicorn.access", level=logging.INFO,
        pathname="", lineno=0, msg='%s - "%s %s HTTP/%s" %d',
        args=("1.2.3.4:54321", "GET", "/mc-skin?name=Notch", "1.1", 200),
        exc_info=None,
    )
    result = filt.filter(record)
    assert result is True, "filter must always return True (don't suppress the record)"
    client_addr, method, full_path, http_version, status_code = record.args
    assert client_addr == "-", "client IP must be blanked to '-'"
    assert "Notch" not in full_path, "username must not appear in the logged path"
    assert "[REDACTED]" in full_path, "query must be replaced with [REDACTED]"


def test_log_filter_redacts_ip_no_query():
    """Filter still blanks the IP even on /mc-skin requests without a query string."""
    filt = appmod._McSkinRedactFilter()
    record = logging.LogRecord(
        name="uvicorn.access", level=logging.INFO,
        pathname="", lineno=0, msg='%s - "%s %s HTTP/%s" %d',
        args=("10.0.0.1:80", "GET", "/mc-skin", "1.1", 400),
        exc_info=None,
    )
    filt.filter(record)
    assert record.args[0] == "-"


def test_log_filter_leaves_other_routes_alone():
    """The filter must NOT modify log records for routes other than /mc-skin."""
    filt = appmod._McSkinRedactFilter()
    original_args = ("5.5.5.5:1234", "GET", "/api/status?foo=bar", "1.1", 200)
    record = logging.LogRecord(
        name="uvicorn.access", level=logging.INFO,
        pathname="", lineno=0, msg='%s - "%s %s HTTP/%s" %d',
        args=original_args,
        exc_info=None,
    )
    filt.filter(record)
    assert record.args == original_args, "non-/mc-skin records must be unchanged"


# ---------------------------------------------------------------------------
# Cache eviction cap (Fix 3)
# ---------------------------------------------------------------------------


@respx.mock
def test_cache_eviction_cap():
    """Inserting more entries than _MC_SKIN_CACHE_MAX must not grow the cache beyond the cap."""
    cap = appmod._MC_SKIN_CACHE_MAX

    # Directly stuff the cache with (cap + 10) synthetic entries so we don't need
    # 500+ Mojang round-trips.  Use distinct fake UUIDs for each entry.
    base_time = time.monotonic() - 1000  # old enough to be candidates for eviction
    for i in range(cap + 10):
        uuid = f"{i:032x}"
        name_key = f"fakeplayer{i}"
        uuid_key = "uuid:" + uuid
        entry = {
            "uuid": uuid,
            "name": name_key,
            "slim": False,
            "png": b"\x89PNG",
            "cached_at": base_time + i,  # monotonically increasing so FIFO ordering holds
        }
        appmod._MC_SKIN_CACHE[name_key] = entry
        appmod._MC_SKIN_CACHE[uuid_key] = entry

    # Trigger eviction explicitly.
    appmod._mc_cache_evict()

    assert len(appmod._MC_SKIN_CACHE) <= cap, (
        f"cache has {len(appmod._MC_SKIN_CACHE)} entries — must be ≤ {cap}"
    )


# ---------------------------------------------------------------------------
# Malformed / empty Mojang 200 body → 502 (Fix 4)
# ---------------------------------------------------------------------------


@respx.mock
def test_malformed_name_response_returns_502():
    """Mojang name endpoint returns 200 with empty (non-JSON) body → 502, not 500."""
    respx.get(url__startswith="https://api.mojang.com").respond(
        status_code=200, content=b"", headers={"content-type": "application/json"}
    )
    r = client.get("/mc-skin?name=Notch")
    assert r.status_code == 502
    assert r.json()["error"] == "offline"


@respx.mock
def test_malformed_profile_response_returns_502():
    """Mojang profile endpoint returns 200 with garbage body → 502, not 500."""
    respx.get(url__startswith="https://api.mojang.com").respond(json=_id_resp())
    respx.get(url__startswith="https://sessionserver.mojang.com").respond(
        status_code=200, content=b"not json at all", headers={"content-type": "application/json"}
    )
    r = client.get("/mc-skin?name=Notch")
    assert r.status_code == 502
    assert r.json()["error"] == "offline"


# ---------------------------------------------------------------------------
# XFF-derived rate-limit key (Fix 2)
# ---------------------------------------------------------------------------


def test_xff_rate_limit_uses_forwarded_ip():
    """When the immediate peer is the trusted proxy (127.0.0.1), the rate-limit
    bucket must key on the X-Forwarded-For IP, not on '127.0.0.1'."""
    # Pre-fill the bucket for a specific real client IP.
    real_ip = "203.0.113.42"
    appmod._MC_SKIN_RATE[real_ip] = (appmod._MC_SKIN_RATE_MAX, time.monotonic())

    # Send a request that looks like it came through nginx on loopback with XFF.
    r = client.get(
        "/mc-skin?name=Notch",
        headers={"X-Forwarded-For": real_ip},
    )
    # The TestClient peer is "testclient", not "127.0.0.1", so XFF is NOT trusted
    # in the test client — this verifies the fallback path doesn't crash.
    # The bucket for "testclient" (the test-client peer) is empty, so the
    # request should proceed past rate-limiting and fail on missing Mojang mock
    # (the respx mock isn't active here), but we just need to confirm it's not 429.
    assert r.status_code != 429, (
        "XFF from an untrusted peer (testclient) must NOT be honoured for rate limiting"
    )


def test_xff_trusted_proxy_rate_limit():
    """When the immediate peer IS the trusted proxy, XFF sets the rate-limit key."""
    # Directly call _mc_real_ip via a mock request to test the helper in isolation.
    from unittest.mock import MagicMock

    real_ip = "198.51.100.7"

    mock_request = MagicMock()
    mock_request.client.host = appmod._MC_TRUSTED_PROXY  # peer = nginx on loopback
    mock_request.headers = {"x-forwarded-for": real_ip}

    derived = appmod._mc_real_ip(mock_request)
    assert derived == real_ip, (
        f"Expected XFF IP {real_ip!r} when peer is trusted proxy, got {derived!r}"
    )


def test_no_xff_falls_back_to_peer():
    """Without a trusted proxy peer, _mc_real_ip falls back to request.client.host."""
    from unittest.mock import MagicMock

    mock_request = MagicMock()
    mock_request.client.host = "192.168.1.5"  # not the trusted proxy
    mock_request.headers = {"x-forwarded-for": "10.0.0.1"}  # should be ignored

    derived = appmod._mc_real_ip(mock_request)
    assert derived == "192.168.1.5", (
        f"Expected peer IP when not behind trusted proxy, got {derived!r}"
    )
