# More games: automatic release downloads

The marketing site's `/more-games` page follows the published Android releases
of Stampede, Twang Thang and Cluster Flock. A game release no longer requires editing the
website or deploying it again.

`tools/sites/marketing/releases.py` owns the catalogue. The FastAPI lifespan
starts a background refresh immediately and repeats it every five minutes.
One bounded, anonymous request per public relay asks for kind 30063 events
from the three pinned release keys on their respective `d` channels. Requests
from website visitors only read the in-memory catalogue; their addresses,
identities and browsing activity are never passed to a relay. There is no
browser relay connection or additional JavaScript.

An announcement must have a valid NIP-01 event ID and BIP-340 signature, the
game's pinned release key, channel and APK signing certificate, and well-formed
version, versionCode, SHA-256, size and canonical HTTPS Blossom URLs. The
highest versionCode wins; its timestamp breaks a tie. An older release never
replaces a newer known one. Future-dated, forged, oversized or malformed events
are ignored. Before adopting a new artifact the server downloads it without
redirects and verifies the exact announced size and hash, and it keeps the
verified file (see "Serving the files" below). It tries the other signed
mirrors if one fails. No signing key is held by the website.

The fixed catalogue entries provide offline bootstrap downloads. Accepted
signed announcements and their verified URL are saved atomically in
`tools/sites/marketing/data/more-games.json`. Startup checks the signatures and
pins again before using the cache, with the bundled version as a floor. Failed
relay or download checks retain the previous downloads. A temporary cache
write failure retains the in-memory release and retries persistence next time.
A complete refresh is bounded to 90 seconds; relay reads and downloads have
their own tighter bounds.

## Serving the files

The public Blossom host answers these APKs as `application/zip` with an inline
`.zip` filename, so a phone saves a `.zip`. The site therefore serves the
verified bytes itself.

- **Storage.** Each verified APK is kept content-addressed at
  `tools/sites/marketing/data/apks/<sha256>.apk`, beside `more-games.json`. The
  download streams to a temporary file in that directory and is renamed into
  place only when its size and SHA-256 both match what was announced; otherwise
  the temporary file is deleted. The final name never holds a partial or
  mismatched file, including when the 90-second refresh bound cancels a download.
  The existing size cap still applies.
- **Verification.** A file is served only if it is a regular file (never a
  symlink), has the announced size, and its SHA-256 has been checked in the
  running process. Files saved by the downloader are remembered at once. Files
  already on disk (after a restart, say) are hashed once by the refresh, off the
  event loop, and remembered by hash, size, modification time and inode, so a
  file changed afterwards stops being served. A file that fails the check is
  deleted and fetched again. Requests never hash; an unverified file means the
  302 fallback below.
- **Filling.** After the new-release loop, any game whose current hash has no
  verified local file (the bundled floor entries, or a cache written before this
  change) is downloaded from its current URL and verified the same way. A
  same-artifact announcement does not download again when the verified file is
  already present. This step also runs when every relay is unreachable. A
  failure is logged and retried at the next refresh; the previous state is kept.
- **Pruning.** Pruning runs at the start of every refresh, before any download,
  and again at the end, so a refresh cut short by the 90-second bound still
  prunes. It removes `data/apks/` files named `<64 hex>.apk` whose hash is
  neither a game's current hash nor one that was current when the previous
  refresh finished (one refresh, about five minutes, of grace, so a just-replaced
  file is not deleted under a running download). It also removes stray temporary
  files older than five minutes. Anything else in the directory is left alone.
- **Route.** `GET /more-games/<slug>.apk` (`stampede`, `twang-thang`,
  `cluster-flock`; the same slugs as the page anchors) returns the file with
  `Content-Type: application/vnd.android.package-archive`,
  `Content-Disposition: attachment; filename="<Name>-<version>.apk"` (for example
  `TwangThang-0.8.22.apk`; the version is reduced to letters, digits, `.`, `_`
  and `-`), `X-Content-Type-Options: nosniff` and `Cache-Control: no-cache`.
  An unknown slug is a 404. When no verified file is stored (or it vanishes
  before it is opened), the route answers 302 to the game's current public URL,
  so the button never breaks. The route opens the file without following
  symlinks and streams from that open handle, so a prune or replace during a
  download cannot fail it; it does not support range requests. The route
  adds no logging; the site still keeps no request logs or IP addresses.
- **Page.** Each "Download APK" button links to that route. The SHA-256 line
  stays so a visitor can check the file.

The `data/` directory is ignored by Git and excluded from deployment rsync,
so future website deployments preserve newer releases and the stored APKs
(about 12 MB each, three current files). The marketing service's `deploy` user
needs write access to `data/` and `data/apks/`, as for the other sites' local
state. No new timer, deployment credential or game-publisher hook is
needed. The game publishers continue to upload and announce releases as they
already do. Neither game's signing key or certificate changes.

For a manual refresh or a deployment check, run from the marketing directory:

```sh
.venv/bin/python releases.py
```

This refreshes the persistent cache, stores and prunes the APKs and prints public
download metadata. Check that `data/apks/` holds one file per game named after
the printed hashes, and that `/more-games` shows the same versions and hashes. Tests use a real
published Stampede signature, independent signed test events, a local relay,
mock download streams, atomic-save, pruning and route checks, persistence/restart checks and the rendered page:

```sh
.venv/bin/python -m pytest -q
```

This directory lists the three vendor games' public installers only. It does not
index player-run servers, host game traffic, or collect player data.
