"""Verified Android releases for /more-games; no visitor makes a relay request."""

import asyncio
import hashlib
import json
import logging
import os
import re
import tempfile
import time
import unicodedata
from urllib.parse import urlsplit
from pathlib import Path

import httpx
from coincurve import PublicKeyXOnly
from websockets.asyncio.client import connect
from websockets.exceptions import ConnectionClosed

log = logging.getLogger(__name__)
REFRESH_SECONDS = 300
RELAY_TIMEOUT = 8
MAX_EVENT_BYTES = 64 * 1024
MAX_APK_BYTES = 256 * 1024 * 1024
RELAYS = ("wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net")
HEX64 = re.compile(r"[0-9a-f]{64}\Z")
BLOSSOM_URL = re.compile(r"https://[A-Za-z0-9.-]+(?::[0-9]{1,5})?/([0-9a-f]{64})(?:\.[A-Za-z0-9]{1,8})?\Z")

# Known downloads remain usable offline. Keys and certificates match the games'
# in-app catalogues; versions here are only the initial floor, never a release step.
MORE_GAMES = [
    {
        "name": "Stampede",
        "tagline": "Laser-eyed ostrich racing: an arcade kart racer.",
        "package": "org.decented.stampede",
        "channel": "stampede-android-apk",
        "pubkey": "b754ca04fd9c09d17b8e12394ef67544475025bc446ac0e55267588c231288a8",
        "cert": "eacd24505a50630e2a115226fe592518bc16cc2421cfc019372b58baeaf08aa9",
        "version": "0.4.14", "version_code": 4014,
        "sha256": "1dbc3656b7e14b2bab82b131a571949830840f8347e9299ead2960dcef4c972e",
        "size_bytes": 12753770,
        "icon": "/static/games/stampede.png",
    },
    {
        "name": "Twang Thang",
        "tagline": "A neon-space energy-band party game.",
        "package": "org.decented.twangthang",
        "channel": "twangthang-android-apk",
        "pubkey": "2cc65a54774d9c1d6fde7e98255dbe1064940facb10dbc3e49f63300a27178c7",
        "cert": "ad02f430148557fb6501c97b16a9ef60681d12d533f7579035e6d62a83dec640",
        "version": "0.8.18", "version_code": 8018,
        "sha256": "cfbe2fabca932f6c0e1d2d13afe126879d0c912faa90fa77ab9f0f026dc22926",
        "size_bytes": 12045165,
        "icon": "/static/games/twangthang.png",
    },
]
for _game in MORE_GAMES:
    _game["url"] = f"https://nostr.download/{_game['sha256']}.apk"


def parse_release(event: dict, game: dict, now: int | None = None) -> dict | None:
    """Accept only this game's signed channel, certificate and canonical APK URLs."""
    if not isinstance(event, dict):
        return None
    try:
        if len(json.dumps(event, ensure_ascii=False).encode()) > MAX_EVENT_BYTES:
            return None
    except (TypeError, ValueError, UnicodeError):
        return None
    if event.get("kind") != 30063 or event.get("pubkey") != game["pubkey"]:
        return None
    created = event.get("created_at")
    if type(created) is not int or not 0 < created <= (int(time.time()) if now is None else now) + 300:
        return None
    if not isinstance(event.get("content"), str):
        return None
    tags = event.get("tags")
    if not isinstance(tags, list) or len(tags) > 128:
        return None
    if any(not isinstance(t, list) or not t or not all(isinstance(s, str) for s in t) for t in tags):
        return None
    fields = {}
    for name in ("d", "version", "version_code", "x", "size", "cert"):
        values = [t[1] for t in tags if len(t) >= 2 and t[0] == name]
        if len(values) != 1:
            return None
        fields[name] = values[0]
    if fields["d"] != game["channel"] or fields["cert"] != game["cert"]:
        return None
    version = fields["version"]
    if not 1 <= len(version.encode()) <= 32 or any(unicodedata.category(c) == "Cc" for c in version):
        return None
    if not HEX64.fullmatch(fields["x"]):
        return None
    for name, maximum in (("version_code", 2**32 - 1), ("size", MAX_APK_BYTES)):
        value = fields[name]
        if not re.fullmatch(r"[0-9]{1,18}", value) or not 0 < int(value) <= maximum:
            return None
    urls = [t[1] for t in tags if len(t) >= 2 and t[0] == "url"]
    if not urls or len(urls) > 8:
        return None
    for url in urls:
        match = BLOSSOM_URL.fullmatch(url)
        if not match or match[1] != fields["x"]:
            return None
        try:
            port = urlsplit(url).port
            if port is not None and not 1 <= port <= 65535:
                return None
        except ValueError:
            return None
    event_id, signature = event.get("id"), event.get("sig")
    if not isinstance(event_id, str) or not HEX64.fullmatch(event_id):
        return None
    if not isinstance(signature, str) or not re.fullmatch(r"[0-9a-f]{128}", signature):
        return None
    canonical = [0, event["pubkey"], created, event["kind"], tags, event["content"]]
    digest = hashlib.sha256(json.dumps(canonical, ensure_ascii=False, separators=(",", ":")).encode()).digest()
    if digest.hex() != event_id:
        return None
    try:
        # Exact lengths are checked before handing any bytes to libsecp256k1.
        key = bytes.fromhex(game["pubkey"])
        if len(key) != 32 or not PublicKeyXOnly(key).verify(bytes.fromhex(signature), digest):
            return None
    except ValueError:
        return None
    return {"version": version, "version_code": int(fields["version_code"]),
            "sha256": fields["x"], "size_bytes": int(fields["size"]),
            "urls": urls, "created_at": created, "event": event}


async def relay_events(url: str, games: list[dict]) -> list[dict]:
    """One bounded, anonymous NIP-01 query. Only configured release keys are requested."""
    events = []
    query = {"kinds": [30063], "authors": [g["pubkey"] for g in games],
             "#d": [g["channel"] for g in games], "limit": 20}
    try:
        async with asyncio.timeout(RELAY_TIMEOUT):
            async with connect(url, open_timeout=3, close_timeout=0.5, max_size=MAX_EVENT_BYTES,
                               max_queue=4, compression=None, ping_interval=None) as ws:
                await ws.send(json.dumps(["REQ", "more-games", query]))
                for _ in range(64):
                    try:
                        frame = json.loads(await ws.recv())
                    except (ValueError, UnicodeError):
                        continue
                    if not isinstance(frame, list) or len(frame) < 2 or frame[1] != "more-games":
                        continue
                    if frame[0] in ("EOSE", "CLOSED"):
                        break
                    if frame[0] == "EVENT" and len(frame) == 3 and isinstance(frame[2], dict):
                        events.append(frame[2])
                await ws.send(json.dumps(["CLOSE", "more-games"]))
    except (TimeoutError, ConnectionClosed):
        # Some relays send events without EOSE. Keep those answers at the deadline.
        if not events:
            raise
    return events


async def fetch_events(games: list[dict]) -> list[dict]:
    replies = await asyncio.gather(*(relay_events(url, games) for url in RELAYS), return_exceptions=True)
    if all(isinstance(reply, BaseException) for reply in replies):
        raise OSError("No release relay answered")
    return [event for reply in replies if isinstance(reply, list) for event in reply]


async def verified_url(release: dict) -> str | None:
    """A new download must actually serve the announced bytes before it is linked."""
    async with httpx.AsyncClient(follow_redirects=False, timeout=20) as client:
        for url in release["urls"]:
            try:
                async with asyncio.timeout(45):
                    async with client.stream("GET", url) as response:
                        if response.status_code != 200:
                            continue
                        size, digest = 0, hashlib.sha256()
                        async for chunk in response.aiter_bytes():
                            size += len(chunk)
                            if size > release["size_bytes"]:
                                break
                            digest.update(chunk)
                        if size == release["size_bytes"] and digest.hexdigest() == release["sha256"]:
                            return url
            except (httpx.HTTPError, TimeoutError):
                continue
    return None


class ReleaseCatalogue:
    def __init__(self, games: list[dict], cache_path: Path):
        self.games = games
        self.dirty = False
        self.cache_path = cache_path
        self.current = {g["package"]: dict(g, created_at=0) for g in games}

    def view(self) -> list[dict]:
        return [dict(self.current[g["package"]]) for g in self.games]

    def rank(self, release: dict) -> tuple[int, int]:
        return release["version_code"], release["created_at"]

    def adopt(self, game: dict, release: dict, url: str) -> None:
        self.dirty = True
        self.current[game["package"]] = dict(game, **{k: v for k, v in release.items() if k != "urls"}, url=url)

    def load(self) -> None:
        try:
            if self.cache_path.stat().st_size > MAX_EVENT_BYTES * len(self.games):
                return
            saved = json.loads(self.cache_path.read_text())
            if not isinstance(saved, dict):
                return
            for game in self.games:
                entry = saved.get(game["package"], {})
                if not isinstance(entry, dict):
                    continue
                release = parse_release(entry.get("event"), game)
                if release and entry.get("url") in release["urls"] and self.rank(release) > self.rank(self.current[game["package"]]):
                    self.adopt(game, release, entry["url"])
            self.dirty = False
        except (OSError, ValueError, UnicodeError):
            log.debug("No readable release cache; using bundled downloads")

    def save(self) -> None:
        saved = {key: {"event": value["event"], "url": value["url"]}
                 for key, value in self.current.items() if "event" in value}
        temporary = None
        try:
            self.cache_path.parent.mkdir(parents=True, exist_ok=True)
            with tempfile.NamedTemporaryFile(mode="w", dir=self.cache_path.parent, delete=False) as f:
                temporary = f.name
                json.dump(saved, f, ensure_ascii=False)
                f.write("\n")
            os.replace(temporary, self.cache_path)
            self.dirty = False
        except OSError:
            log.warning("Could not persist release links; in-memory downloads remain available")
        finally:
            if temporary and os.path.exists(temporary):
                os.unlink(temporary)

    async def refresh(self) -> None:
        if self.dirty:
            self.save()
        events = await fetch_events(self.games)
        for game in self.games:
            candidates = [r for e in events if (r := parse_release(e, game)) is not None]
            candidates.sort(key=self.rank, reverse=True)
            current = self.current[game["package"]]
            for release in candidates:
                if self.rank(release) <= self.rank(current):
                    break
                same_artifact = (release["sha256"], release["size_bytes"]) == (current["sha256"], current["size_bytes"])
                url = current["url"] if same_artifact and current["url"] in release["urls"] else await verified_url(release)
                if url:
                    self.adopt(game, release, url)
                    self.save()
                    break

    async def run(self) -> None:
        while True:
            try:
                async with asyncio.timeout(90):
                    await self.refresh()
            except Exception:
                log.warning("Release refresh unavailable; retaining verified download links")
            await asyncio.sleep(REFRESH_SECONDS)


if __name__ == "__main__":
    catalogue = ReleaseCatalogue(MORE_GAMES, Path(__file__).parent / "data" / "more-games.json")
    catalogue.load()
    asyncio.run(catalogue.refresh())
    print(json.dumps([{k: g[k] for k in ("name", "version", "url", "sha256", "size_bytes")} for g in catalogue.view()], indent=2))
