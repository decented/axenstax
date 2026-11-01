"""The /mc-skin in-memory stores (IP rate buckets, typed-username cache) are
swept on a timer, not only lazily, so nothing lingers until a restart.

Run: cd tools/sites/game && .venv/bin/python -m pytest test_mc_skin_sweep.py -q
"""

import os
import sys
import time

import pytest
from starlette.testclient import TestClient

sys.path.insert(0, os.path.dirname(__file__))

import app as appmod  # noqa: E402


@pytest.fixture(autouse=True)
def clear_mc_state():
    appmod._MC_SKIN_CACHE.clear()
    appmod._MC_SKIN_RATE.clear()
    yield
    appmod._MC_SKIN_CACHE.clear()
    appmod._MC_SKIN_RATE.clear()


def _entry(uuid: str, cached_at: float) -> dict:
    return {"uuid": uuid, "name": "Steve", "slim": False, "png": b"x", "cached_at": cached_at}


def _put_pair(name_key: str, uuid: str, cached_at: float) -> None:
    e = _entry(uuid, cached_at)
    appmod._MC_SKIN_CACHE[name_key] = e
    appmod._MC_SKIN_CACHE["uuid:" + uuid.lower()] = e


def test_sweep_removes_rate_entries_once_their_window_has_passed():
    now = 10_000.0
    appmod._MC_SKIN_RATE["old"] = (3, now - appmod._MC_SKIN_RATE_WINDOW)  # exactly expired
    appmod._MC_SKIN_RATE["older"] = (1, now - 500.0)
    appmod._MC_SKIN_RATE["fresh"] = (2, now - appmod._MC_SKIN_RATE_WINDOW + 1.0)
    appmod._mc_sweep_expired(now)
    assert set(appmod._MC_SKIN_RATE) == {"fresh"}


def test_sweep_removes_cache_entries_once_their_ttl_has_passed_name_and_alias_together():
    now = 10_000.0
    uuid_old = "a" * 32
    uuid_new = "B" * 32
    _put_pair("steve", uuid_old, now - appmod._MC_SKIN_CACHE_TTL)  # exactly expired
    _put_pair("alex", uuid_new, now - appmod._MC_SKIN_CACHE_TTL + 1.0)
    appmod._mc_sweep_expired(now)
    assert set(appmod._MC_SKIN_CACHE) == {"alex", "uuid:" + uuid_new.lower()}


def test_sweep_leaves_everything_alone_when_nothing_has_expired():
    now = 10_000.0
    appmod._MC_SKIN_RATE["a"] = (1, now - 1.0)
    _put_pair("steve", "c" * 32, now - 1.0)
    appmod._mc_sweep_expired(now)
    assert len(appmod._MC_SKIN_RATE) == 1
    assert len(appmod._MC_SKIN_CACHE) == 2


def test_sweep_is_safe_on_empty_stores():
    appmod._mc_sweep_expired(1.0)
    assert not appmod._MC_SKIN_RATE and not appmod._MC_SKIN_CACHE


def test_sweep_uses_the_monotonic_clock_by_default():
    appmod._MC_SKIN_RATE["old"] = (1, time.monotonic() - 1000.0)
    appmod._MC_SKIN_RATE["fresh"] = (1, time.monotonic())
    appmod._mc_sweep_expired()
    assert set(appmod._MC_SKIN_RATE) == {"fresh"}


def test_sweep_interval_is_sixty_seconds():
    assert appmod._MC_SWEEP_INTERVAL == 60.0


def _wait_until(pred, timeout=5.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if pred():
            return True
        time.sleep(0.02)
    return pred()


def test_the_sweep_task_runs_under_the_app_lifespan_with_no_request(monkeypatch):
    """Would fail if the startup wiring were removed: entries are planted, no
    request is made, and the lifespan's background task must clear them."""
    monkeypatch.setattr(appmod, "_MC_SWEEP_INTERVAL", 0.05)
    with TestClient(appmod.app):
        appmod._MC_SKIN_RATE["1.2.3.4"] = (1, time.monotonic() - 1000.0)
        _put_pair("steve", "d" * 32, time.monotonic() - 1000.0)
        assert _wait_until(lambda: not appmod._MC_SKIN_RATE and not appmod._MC_SKIN_CACHE)


def test_the_sweep_task_survives_a_failing_sweep_and_stops_on_shutdown(monkeypatch, caplog):
    calls = {"n": 0}
    real = appmod._mc_sweep_expired

    def flaky(now=None):
        calls["n"] += 1
        if calls["n"] == 1:
            raise RuntimeError("boom")
        return real(now)

    monkeypatch.setattr(appmod, "_MC_SWEEP_INTERVAL", 0.05)
    monkeypatch.setattr(appmod, "_mc_sweep_expired", flaky)
    with TestClient(appmod.app):
        appmod._MC_SKIN_RATE["1.2.3.4"] = (1, time.monotonic() - 1000.0)
        assert _wait_until(lambda: not appmod._MC_SKIN_RATE)
        assert calls["n"] >= 2
    n_after_shutdown = calls["n"]
    time.sleep(0.2)
    assert calls["n"] == n_after_shutdown
    # the failure is logged without any name, uuid or address
    assert all("1.2.3.4" not in r.getMessage() for r in caplog.records)
