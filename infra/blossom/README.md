# Blossom blob store — AxeNStax cloud-save bridge tier

## What this is

A [Blossom](https://github.com/hzrd149/blossom-server) blob store (BUD-01/02/06)
backing the AxeNStax **cloud-save "bridge tier"**. The game site
(`tools/sites/game`) proxies world-blob uploads and downloads to it
server-side.

It binds **loopback only** (`127.0.0.1:3000`) because the game site reaches it
over loopback from the same host. It is **NOT exposed publicly** — there is no
public ingress, no TLS terminator, nothing forwarding to port 3000 from
outside.

## Why server-key signing (BRIDGE)

The game site signs Blossom `kind-24242` auth events with a **server key** on
the player's behalf, rather than the player's own key. This is a deliberate
**bridge**: the destination tier will switch to player-key signing once the
Signet SDK exposes `sign_event`. Until then, one server-held key authenticates
all uploads/lists against this store.

Cross-reference: `docs/foundations/2026-05-26-cloud-save-blossom.md`.

## Config schema caveat

The keys in `config.yml` target the `ghcr.io/hzrd149/blossom-server` image, and
that schema **can shift between versions**. Before first run, verify the keys
against the pinned image's `config.example.yml`:

```bash
docker run --rm ghcr.io/hzrd149/blossom-server:master cat config.example.yml
```

(or check the upstream repo) and adjust `config.yml` if the schema differs.

## Generating the server key (the whitelist value)

From `tools/sites/game/`, run:

```bash
./.venv/bin/python -c "import os; from dotenv import load_dotenv; load_dotenv(); from nostr_auth import server_keys; server_keys.load_or_generate(); print('pubkey', server_keys.pubkey_hex); print('secret', server_keys.privkey_hex)"
```

If `NOSTR_SERVER_KEY` is unset in `tools/sites/game/.env`, this prints a
**freshly generated** key — copy `secret` into `.env` as
`NOSTR_SERVER_KEY=<secret>` so it persists, then re-run to confirm a **stable**
`pubkey`.

Put that `pubkey` (hex) into `config.yml` under `upload.pubkeys`, replacing
`REPLACE_WITH_BLOSSOM_APP_PUBKEY_HEX`.

## Game-site env

The game site needs two env vars in `tools/sites/game/.env`:

```
BLOSSOM_URL=http://127.0.0.1:3000
NOSTR_SERVER_KEY=<secret>
```

Without them, cloud save is **inert** — the lobby shows `cloud-save: disabled`.

## Reads are public

`blossom-server` serves GET (download) without auth by default, which matches
the game site's `_blossom_get` (no auth header). Only **upload** and **list**
require the `kind-24242` auth event.

If you flip read-auth on, the game site's `_blossom_get` must be changed to send
`Authorization: Nostr <server_keys.sign_blossom_get(hash)>` (the signer method
already exists).

## Run it

```bash
infra/blossom/start.sh   # bring up
infra/blossom/stop.sh    # stop
```

The named volume `blossom-data` persists blobs across restarts. (Requires Docker
+ the compose plugin on the host.)

## Production note

This is the **local/dev stand-up**. Production containerisation (k8s Deployment
+ PVC, sibling to the trotters relay) is deferred — not in scope here.
