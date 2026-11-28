"""Verified Android releases for /more-games; no visitor makes a relay request."""

import asyncio
import hashlib
import json
import logging
import os
import re
import stat
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
RELAYS = ("wss://relay.damus.io", "wss://relay.ditto.pub", "wss://relay.primal.net")
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
    {
        "name": "Cluster Flock",
        "tagline": "Three-lane ostrich farm run, with ghost saboteurs.",
        "package": "org.decented.gameflock",
        "channel": "gameflock-android-apk",
        "pubkey": "01b0f270e290f5cd48aa537392de04e4b22124c16d696de96158395a60343b0f",
        "cert": "46fd4f7a80b700cd7d951c04e38fa3344405759d53d161de3e4dd06120fb3ace",
        "version": "0.2.1", "version_code": 2001,
        "sha256": "893218af0edef806a192535cda51b8b6eba695673ea13fe3a10457fa75a61849",
        "size_bytes": 10931051,
        "icon": "/static/games/gameflock.png",
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


APK_FILE = re.compile(r"[0-9a-f]{64}\.apk\Z")
TEMP_PREFIX, TEMP_SUFFIX = ".apk-", ".part"
STALE_TEMP_SECONDS = 300


async def download_apk(release: dict, apk_dir: Path) -> str | None:
    """Fetch the announced bytes, check size and SHA-256, and only then keep them.

    The file lands at ``<apk_dir>/<sha256>.apk`` via an atomic rename, so the final
    name never holds a partial or mismatched file. Returns the URL that served the
    bytes, or None when no URL did.
    """
    sha, expected = release["sha256"], release["size_bytes"]
    if not HEX64.fullmatch(sha) or not 0 < expected <= MAX_APK_BYTES:
        return None
    try:
        apk_dir.mkdir(parents=True, exist_ok=True)
    except OSError:
        log.warning("Could not create the download directory")
        return None
    async with httpx.AsyncClient(follow_redirects=False, timeout=20) as client:
        for url in release["urls"]:
            try:
                async with asyncio.timeout(45):
                    if await _fetch_to_file(client, url, sha, expected, apk_dir):
                        return url
            except (httpx.HTTPError, TimeoutError, OSError):
                continue
    return None


async def _fetch_to_file(client: httpx.AsyncClient, url: str, sha: str, expected: int, apk_dir: Path) -> bool:
    handle, temporary = tempfile.mkstemp(dir=apk_dir, prefix=TEMP_PREFIX, suffix=TEMP_SUFFIX)
    try:
        with os.fdopen(handle, "wb") as out:
            async with client.stream("GET", url) as response:
                if response.status_code != 200:
                    return False
                size, digest = 0, hashlib.sha256()
                async for chunk in response.aiter_bytes():
                    size += len(chunk)
                    if size > expected:
                        return False
                    digest.update(chunk)
                    out.write(chunk)
        if size != expected or digest.hexdigest() != sha:
            return False
        os.chmod(temporary, 0o644)
        os.replace(temporary, apk_dir / f"{sha}.apk")
        temporary = None
        return True
    finally:
        # Also runs when the 90 s refresh bound cancels a download mid-stream.
        if temporary is not None:
            try:
                os.unlink(temporary)
            except OSError:
                pass


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1 << 20):
            digest.update(chunk)
    return digest.hexdigest()


class ReleaseCatalogue:
    def __init__(self, games: list[dict], cache_path: Path, apk_dir: Path | None = None):
        self.games = games
        self.dirty = False
        self.cache_path = cache_path
        self.apk_dir = apk_dir if apk_dir is not None else cache_path.parent / "apks"
        # (sha256, size, mtime_ns, inode) of stored files whose bytes were hashed in this
        # process. Requests only consult it; they never hash.
        self._verified: set[tuple] = set()
        # Hashes that were current when the previous refresh finished (prune grace).
        self._recent: set[str] = set()
        self.current = {g["package"]: dict(g, created_at=0) for g in games}

    def view(self) -> list[dict]:
        return [dict(self.current[g["package"]]) for g in self.games]

    def rank(self, release: dict) -> tuple[int, int]:
        return release["version_code"], release["created_at"]

    @staticmethod
    def _key(sha: str, info: os.stat_result) -> tuple:
        return sha, info.st_size, info.st_mtime_ns, info.st_ino

    def _file_for(self, release: dict) -> Path | None:
        """The stored copy of this release's bytes: a regular file (never a symlink)
        of the announced size whose hash this process has verified. Never hashes."""
        sha = release["sha256"]
        path = self.apk_dir / f"{sha}.apk"
        try:
            info = path.lstat()
        except OSError:
            return None
        if not stat.S_ISREG(info.st_mode) or info.st_size != release["size_bytes"]:
            return None
        return path if self._key(sha, info) in self._verified else None

    def local_apk(self, package: str) -> Path | None:
        """The current release's verified file on this server, or None."""
        current = self.current.get(package)
        return self._file_for(current) if current else None

    def open_apk(self, package: str):
        """Open the current release's verified file for streaming, or None.

        Opened without following symlinks and checked against the verified memo by
        the open handle's own identity, so a file swapped or removed after the check
        cannot be served, and pruning an open file cannot break a running download.
        """
        current = self.current.get(package)
        if current is None:
            return None
        path = self.apk_dir / f"{current['sha256']}.apk"
        try:
            fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        except OSError:
            return None
        try:
            info = os.fstat(fd)
            if (not stat.S_ISREG(info.st_mode) or info.st_size != current["size_bytes"]
                    or self._key(current["sha256"], info) not in self._verified):
                os.close(fd)
                return None
            return os.fdopen(fd, "rb"), info.st_size
        except OSError:
            os.close(fd)
            return None

    def _remember(self, release: dict) -> None:
        """Trust a file that download_apk has just hashed and renamed into place."""
        try:
            info = (self.apk_dir / f"{release['sha256']}.apk").lstat()
        except OSError:
            return
        if stat.S_ISREG(info.st_mode) and info.st_size == release["size_bytes"]:
            self._verified.add(self._key(release["sha256"], info))

    async def verify_stored(self) -> None:
        """Hash each current release's stored file once (off the event loop). A symlink,
        wrong-size or wrong-hash file is deleted so the fill step fetches it afresh."""
        for current in list(self.current.values()):
            path = self.apk_dir / f"{current['sha256']}.apk"
            try:
                info = path.lstat()
            except OSError:
                continue
            key = self._key(current["sha256"], info)
            if key in self._verified:
                continue
            good = False
            if stat.S_ISREG(info.st_mode) and info.st_size == current["size_bytes"]:
                try:
                    good = await asyncio.to_thread(_sha256_file, path) == current["sha256"]
                    after = path.lstat()
                    good = good and self._key(current["sha256"], after) == key
                except OSError:
                    good = False
            if good:
                self._verified.add(key)
                continue
            log.warning("Stored download for %s failed its check; replacing it", current["name"])
            try:
                path.unlink()
            except OSError:
                pass

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
        # First, so a refresh cut short by the 90 s bound still prunes.
        self.prune()
        await self.verify_stored()
        events, outage = [], None
        try:
            events = await fetch_events(self.games)
        except OSError as error:
            # Stored files can still be topped up and pruned while the relays are down.
            outage = error
        for game in self.games:
            candidates = [r for e in events if (r := parse_release(e, game)) is not None]
            candidates.sort(key=self.rank, reverse=True)
            current = self.current[game["package"]]
            for release in candidates:
                if self.rank(release) <= self.rank(current):
                    break
                same_artifact = (release["sha256"], release["size_bytes"]) == (current["sha256"], current["size_bytes"])
                if same_artifact and current["url"] in release["urls"]:
                    url = current["url"]
                elif self._file_for(release):
                    url = release["urls"][0]  # bytes already verified on disk: no second download
                else:
                    url = await download_apk(release, self.apk_dir)
                    if url:
                        self._remember(release)
                if url:
                    self.adopt(game, release, url)
                    self.save()
                    break
        await self.fill_missing()
        self.prune()
        self._recent = {g["sha256"] for g in self.current.values()}
        if outage is not None:
            raise outage

    async def fill_missing(self) -> None:
        """Bundled floor entries and caches from older code have no stored file yet."""
        await self.verify_stored()
        for game in self.games:
            current = self.current[game["package"]]
            if self._file_for(current):
                continue
            offer = {"urls": [current["url"]], "sha256": current["sha256"], "size_bytes": current["size_bytes"]}
            if await download_apk(offer, self.apk_dir):
                self._remember(offer)
            else:
                log.warning("Could not store %s %s locally; its button redirects to the public host",
                            game["name"], current["version"])

    def prune(self) -> None:
        """Remove stored APKs no game offers now or offered at the last refresh, and stray temporary files."""
        # One refresh of grace: a file that has just been superseded may still be
        # open in a running download.
        keep = {g["sha256"] for g in self.current.values()} | self._recent
        try:
            entries = list(self.apk_dir.iterdir())
        except OSError:
            return
        for path in entries:
            try:
                if not path.is_file() or path.is_symlink():
                    continue
                if APK_FILE.fullmatch(path.name):
                    stale = path.name[:-4] not in keep
                elif path.name.startswith(TEMP_PREFIX) and path.name.endswith(TEMP_SUFFIX):
                    stale = time.time() - path.stat().st_mtime > STALE_TEMP_SECONDS
                else:
                    continue
                if stale:
                    path.unlink()
            except OSError:
                log.debug("Could not prune a stored download")

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
