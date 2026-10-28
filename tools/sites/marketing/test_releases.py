"""Real signature, relay, download, persistence and rendered-page regressions."""

import asyncio
import copy
import hashlib
import json
from pathlib import Path
from unittest.mock import AsyncMock, patch

import httpx
import pytest
from coincurve import PrivateKey
from starlette.testclient import TestClient
from websockets.asyncio.server import serve

import app as appmod
import releases

# Public test key, unrelated to either game's signing keys.
KEY = PrivateKey(bytes.fromhex("01" * 32))
BODY = b"a test APK"
NOW = 1_800_000_000


def game():
    entry = dict(releases.MORE_GAMES[0])
    entry["pubkey"] = KEY.public_key_xonly.format().hex()
    return entry


def signed(code=4015, created=NOW, **changes):
    g = game()
    event = {"pubkey": g["pubkey"], "created_at": created, "kind": 30063,
             "tags": [["d", g["channel"]], ["version", "0.4.15"], ["version_code", str(code)],
                      ["x", hashlib.sha256(BODY).hexdigest()], ["size", str(len(BODY))], ["cert", g["cert"]],
                      ["url", "https://nostr.download/" + hashlib.sha256(BODY).hexdigest() + ".apk"]],
             "content": "A new release — with Unicode 🐦"}
    for name, value in changes.items():
        for tag in event["tags"]:
            if tag[0] == name:
                tag[1] = value
    return sign(event)


def sign(event, key=KEY):
    event = copy.deepcopy(event)
    data = [0, event["pubkey"], event["created_at"], event["kind"], event["tags"], event["content"]]
    digest = hashlib.sha256(json.dumps(data, ensure_ascii=False, separators=(",", ":")).encode()).digest()
    event["id"] = digest.hex()
    event["sig"] = key.sign_schnorr(digest, aux_randomness=bytes(32)).hex()
    return event


def parse(event):
    return releases.parse_release(event, game(), NOW)


def test_real_published_stampede_event_verifies_with_the_pinned_key():
    event = json.loads((Path(__file__).parent / "fixtures/stampede-release.json").read_text())
    release = releases.parse_release(event, releases.MORE_GAMES[0])
    assert release and release["version"] == "0.4.14"
    assert release["sha256"] == "1dbc3656b7e14b2bab82b131a571949830840f8347e9299ead2960dcef4c972e"


def test_signed_release_parses():
    result = parse(signed())
    assert result["version_code"] == 4015 and result["size_bytes"] == len(BODY)


@pytest.mark.parametrize("field,value", [("kind", 1), ("pubkey", "00" * 32), ("id", "00" * 32),
                                        ("sig", "00" * 64), ("sig", "00"), ("created_at", NOW + 301),
                                        ("created_at", True), ("content", "edited")])
def test_tampered_or_malformed_envelopes_are_rejected(field, value):
    event = signed()
    event[field] = value
    assert parse(event) is None


@pytest.mark.parametrize("name,value", [("d", "twangthang-android-apk"), ("cert", "00" * 32),
                                      ("version", ""), ("version", "x" * 33), ("version", "bad\nversion"),
                                      ("version_code", "0"), ("version_code", "4294967296"),
                                      ("version_code", "1e3"), ("version_code", "+1"),
                                      ("size", "0"), ("size", str(releases.MAX_APK_BYTES + 1)),
                                      ("x", "ff"), ("x", "FF" * 32)])
def test_invalid_signed_release_tags_are_rejected(name, value):
    assert parse(signed(**{name: value})) is None


@pytest.mark.parametrize("url", ["http://example.org/{sha}.apk", "https://user@example.org/{sha}.apk",
                                 "https://example.org/folder/{sha}.apk", "https://example.org/{sha}.apk?download=1",
                                 "https://example.org/{sha}.apk#fragment", "https://example.org:0/{sha}.apk",
                                 "https://example.org:65536/{sha}.apk", "https://example.org/" + "00" * 32 + ".apk"])
def test_noncanonical_download_urls_are_rejected(url):
    assert parse(signed(url=url.format(sha=hashlib.sha256(BODY).hexdigest()))) is None


def test_duplicate_critical_tags_and_wrong_signers_are_rejected():
    event = signed()
    event["tags"].append(["version_code", "4016"])
    assert parse(sign(event)) is None
    assert releases.parse_release(signed(), releases.MORE_GAMES[0], NOW) is None


def test_oversized_and_invalid_unicode_events_do_not_break_the_reader():
    event = signed()
    event["content"] = "x" * releases.MAX_EVENT_BYTES
    assert parse(event) is None
    event["content"] = "\ud800"
    assert parse(event) is None
    assert parse({"content": bytes(10)}) is None


def catalogue(tmp_path):
    return releases.ReleaseCatalogue([game()], tmp_path / "data" / "releases.json")


def refresh(cat, events, url=True):
    async def run():
        with patch.object(releases, "fetch_events", AsyncMock(return_value=events)), \
             patch.object(releases, "verified_url", AsyncMock(return_value=parse(signed())["urls"][0] if url else None)) as check, \
             patch.object(releases.time, "time", return_value=NOW):
            await cat.refresh()
            return check.await_count
    return asyncio.run(run())


def test_verified_new_release_updates_the_page_and_survives_restart(tmp_path):
    cat = catalogue(tmp_path)
    assert refresh(cat, [signed()]) == 1
    assert cat.view()[0]["version_code"] == 4015
    restored = catalogue(tmp_path)
    with patch.object(releases.time, "time", return_value=NOW):
        restored.load()
    assert restored.view()[0]["sha256"] == hashlib.sha256(BODY).hexdigest()
    with patch.object(appmod, "catalogue", restored):
        text = TestClient(appmod.app).get("/more-games").text
    assert "Version 0.4.15" in text and restored.view()[0]["url"] in text


def test_bad_download_or_unsigned_event_never_replaces_the_working_link(tmp_path):
    cat = catalogue(tmp_path)
    assert refresh(cat, [signed()], url=False) == 1
    assert cat.view()[0]["version_code"] == 4014
    event = signed()
    event["content"] = "tampered"
    assert refresh(cat, [event]) == 0
    assert cat.view()[0]["version_code"] == 4014


def test_version_code_takes_priority_over_timestamp_and_never_rolls_back(tmp_path):
    cat = catalogue(tmp_path)
    refresh(cat, [signed(4015, NOW), signed(4016, NOW - 10)])
    assert cat.view()[0]["version_code"] == 4016
    assert refresh(cat, [signed(4015, NOW + 100)]) == 0
    assert cat.view()[0]["version_code"] == 4016


def test_relay_outage_keeps_the_last_verified_release(tmp_path):
    cat = catalogue(tmp_path)
    refresh(cat, [signed()])
    async def run():
        with patch.object(releases, "fetch_events", AsyncMock(side_effect=OSError("offline"))):
            with pytest.raises(OSError):
                await cat.refresh()
    asyncio.run(run())
    assert cat.view()[0]["version_code"] == 4015


def test_tampered_cache_and_an_older_cache_cannot_replace_the_seed(tmp_path):
    cat = catalogue(tmp_path)
    refresh(cat, [signed()])
    saved = json.loads(cat.cache_path.read_text())
    saved[game()["package"]]["event"]["content"] = "changed"
    cat.cache_path.write_text(json.dumps(saved))
    restored = catalogue(tmp_path)
    restored.load()
    assert restored.view()[0]["version_code"] == 4014
    refresh(cat, [signed(4016)])
    newer_game = dict(game(), version_code=5000)
    restored = releases.ReleaseCatalogue([newer_game], cat.cache_path)
    with patch.object(releases.time, "time", return_value=NOW):
        restored.load()
    assert restored.view()[0]["version_code"] == 5000


def test_cache_write_failure_does_not_lose_the_new_download(tmp_path):
    cat = catalogue(tmp_path)
    with patch.object(releases.os, "replace", side_effect=OSError("disk full")):
        refresh(cat, [signed()])
    assert cat.view()[0]["version_code"] == 4015
    assert not list(cat.cache_path.parent.iterdir())
    assert cat.dirty
    refresh(cat, [])
    assert cat.cache_path.is_file() and not cat.dirty


def test_background_loop_retries_without_visitors(tmp_path):
    cat = catalogue(tmp_path)
    calls = []
    async def refresh_once():
        calls.append("refresh")
        if len(calls) == 1:
            raise OSError("offline")
    async def sleep_once(delay):
        assert delay == 300
        if len(calls) == 2:
            raise asyncio.CancelledError
    async def run():
        with patch.object(cat, "refresh", refresh_once), patch.object(releases.asyncio, "sleep", sleep_once):
            with pytest.raises(asyncio.CancelledError):
                await cat.run()
    asyncio.run(run())
    assert calls == ["refresh", "refresh"]


def test_relay_query_checks_subscription_and_closes_after_eose():
    async def run():
        async def handler(ws):
            frame = json.loads(await ws.recv())
            assert frame[0:2] == ["REQ", "more-games"]
            assert frame[2]["authors"] == [game()["pubkey"]]
            assert frame[2]["#d"] == [game()["channel"]]
            await ws.send(json.dumps(["EVENT", "unrelated", signed()]))
            await ws.send(json.dumps(["EVENT", "more-games", signed()]))
            await ws.send(json.dumps(["EOSE", "more-games"]))
            assert json.loads(await ws.recv()) == ["CLOSE", "more-games"]
        async with serve(handler, "127.0.0.1", 0) as server:
            port = server.sockets[0].getsockname()[1]
            events = await releases.relay_events(f"ws://127.0.0.1:{port}", [game()])
            assert len(events) == 1
    asyncio.run(run())


def test_one_healthy_relay_is_enough_and_total_failure_is_an_error():
    async def run():
        with patch.object(releases, "relay_events", AsyncMock(side_effect=[OSError(), [signed()], OSError()])):
            assert len(await releases.fetch_events([game()])) == 1
        with patch.object(releases, "relay_events", AsyncMock(side_effect=OSError())):
            with pytest.raises(OSError):
                await releases.fetch_events([game()])
    asyncio.run(run())


@pytest.mark.parametrize("body,status", [(BODY, 200), (BODY + b"extra", 200), (b"wrong", 200), (BODY, 302)])
def test_download_checks_bytes_size_and_redirects(body, status):
    client = httpx.AsyncClient
    transport = httpx.MockTransport(lambda request: httpx.Response(status, content=body))
    def factory(**kwargs):
        return client(transport=transport, **kwargs)
    async def run():
        with patch.object(releases.httpx, "AsyncClient", factory):
            return await releases.verified_url(parse(signed()))
    assert (asyncio.run(run()) is not None) == (body == BODY and status == 200)


def test_app_lifespan_starts_the_reader_and_cancels_it_at_shutdown(tmp_path):
    cat = catalogue(tmp_path)
    refresh(cat, [signed()])
    started, stopped = [], []
    async def run():
        started.append(True)
        try:
            await asyncio.Event().wait()
        finally:
            stopped.append(True)
    with patch.object(appmod, "catalogue", cat), patch.object(cat, "run", run):
        with TestClient(appmod.app) as client:
            assert started and not stopped
            assert "Version 0.4.15" in client.get("/more-games").text
        assert stopped


def test_relay_answers_survive_a_missing_eose():
    async def run():
        async def handler(ws):
            await ws.recv()
            await ws.send(json.dumps(["EVENT", "more-games", signed()]))
            await ws.wait_closed()
        async with serve(handler, "127.0.0.1", 0) as server:
            port = server.sockets[0].getsockname()[1]
            with patch.object(releases, "RELAY_TIMEOUT", 0.05):
                events = await releases.relay_events(f"ws://127.0.0.1:{port}", [game()])
            assert len(events) == 1
    asyncio.run(run())
