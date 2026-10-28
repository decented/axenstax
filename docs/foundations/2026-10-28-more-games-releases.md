# More games: automatic release downloads

The marketing site's `/more-games` page follows the published Android releases
of Stampede and Twang Thang. A game release no longer requires editing the
website or deploying it again.

`tools/sites/marketing/releases.py` owns the catalogue. The FastAPI lifespan
starts a background refresh immediately and repeats it every five minutes.
One bounded, anonymous request per public relay asks for kind 30063 events
from the two pinned release keys on their respective `d` channels. Requests
from website visitors only read the in-memory catalogue; their addresses,
identities and browsing activity are never passed to a relay. There is no
browser relay connection or additional JavaScript.

An announcement must have a valid NIP-01 event ID and BIP-340 signature, the
game's pinned release key, channel and APK signing certificate, and well-formed
version, versionCode, SHA-256, size and canonical HTTPS Blossom URLs. The
highest versionCode wins; its timestamp breaks a tie. An older release never
replaces a newer known one. Future-dated, forged, oversized or malformed events
are ignored. Before adopting a new artifact the server downloads it without
redirects and verifies the exact announced size and hash. It tries the other
signed mirrors if one fails. No signing key is held by the website.

The fixed catalogue entries provide offline bootstrap downloads. Accepted
signed announcements and their verified URL are saved atomically in
`tools/sites/marketing/data/more-games.json`. Startup checks the signatures and
pins again before using the cache, with the bundled version as a floor. Failed
relay or download checks retain the previous downloads. A temporary cache
write failure retains the in-memory release and retries persistence next time.
A complete refresh is bounded to 90 seconds; relay reads and downloads have
their own tighter bounds.

The `data/` directory is ignored by Git and excluded from deployment rsync,
so future website deployments preserve newer releases. The marketing service's
`deploy` user needs write access to that directory, as for the other sites'
local state. No new timer, deployment credential or game-publisher hook is
needed. The game publishers continue to upload and announce releases as they
already do. Neither game's signing key or certificate changes.

For a manual refresh or a deployment check, run from the marketing directory:

```sh
.venv/bin/python releases.py
```

This refreshes the persistent cache and prints public download metadata. Check
that `/more-games` shows the same versions, hashes and URLs. Tests use a real
published Stampede signature, independent signed test events, a local relay,
mock download streams, persistence/restart checks and the rendered page:

```sh
.venv/bin/python -m pytest -q
```

This directory lists the two vendor games' public installers only. It does not
index player-run servers, host game traffic, or collect player data.
