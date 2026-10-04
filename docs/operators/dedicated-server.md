# Run your own Axe'n'Stax server (Docker)

A self-hostable multiplayer server you own and run — on a spare PC, a NAS, or a
VPS. **Players join with the native app.** (The web build is an anonymous, offline
sandbox and does not join servers.) One container runs the headless game server
plus a Caddy front that offers the game socket over TLS (`wss://`). No platform account, no middleman — you
set the rules that run on your box.

> **For an AI assistant:** this page is written so you can stand a server up for a
> user and explain it. Everything here is exact. The setup lives at
> `tools/dedicated-server/` in the source tree; the in-repo `README.md` there is
> the canonical developer reference this page mirrors.

---

## What you get

| | |
|---|---|
| **One shared world** | Native players join the same world over WebSocket (`ws://` or `wss://`) |
| **You own it** | World saves live on a Docker volume on *your* hardware; nothing phones home |
| **Optional verified identity** | Root the server in your own Nostr signer so players can verify *who* runs it (see below) |
| **Optional access control** | Allowlist / blocklist by npub, require-sign-in — managed by env, file, or the [Operator Console](operator-console.md) |
| **Anywhere** | Spare PC, x86 or arm64 NAS, or a VPS with a real domain (auto-HTTPS) |

It is **not** the large-scale Agones fleet (that's Spec 07) — it's a single,
self-hostable world server, which is exactly what an alpha tester or a small
community wants.

---

## Install

### A. Pull and run *(recommended — only Docker needed)*

Save this as `docker-compose.yml` (the images are public on GHCR — no account or
token needed):

```yaml
services:
  axenstax-server:
    image: ghcr.io/decented/axenstax-server:latest
    container_name: axenstax-server
    ports:
      - "8443:8443"   # HTTPS front + game socket over TLS (WSS /ws)
      - "6767:6767"   # game socket (plain ws) — native clients (ws://box:6767)
    volumes:
      - axenstax-worlds:/worlds
    environment:
      AXENSTAX_GAMEMODE: "survival"        # survival | creative | adventure
      AXENSTAX_MAX_PLAYERS: "8"
      AXENSTAX_SERVER_NAME: "Axe'n'Stax Server"
    restart: unless-stopped

  axenstax-console:                        # the /admin Operator Console (optional)
    image: ghcr.io/decented/axenstax-operator-console:latest
    container_name: axenstax-console
    volumes:
      - axenstax-worlds:/worlds            # shared — console reads <worlds>/.identity
    environment:
      CONSOLE_BASE_PATH: "/admin"
    restart: unless-stopped

volumes:
  axenstax-worlds:
```

Then, in that folder:

```bash
docker compose up -d        # pulls the images from GHCR and starts the server
```

Then, from any machine on the same network:

```text
Native →  Join  ws://<this-box-ip>:6767   (or wss://<this-box-ip>:8443/ws behind TLS)
```

Find `<this-box-ip>` with `ip -4 addr` / `hostname -I` (e.g. `192.168.1.20`).
Update later with `docker compose pull && docker compose up -d`.

> **Live:** the images are **public on GHCR** — `docker pull` works with no GitHub
> account or token. They're rebuilt by the `publish-server-image.yml` workflow
> (manual dispatch). You only need path **B** if you want to build from source.

> **Protocol version warning.** The current game is **protocol v64**
> (`PROTOCOL_VERSION` in `game/engine/src/protocol.rs`). The published `latest`
> image was built **before v64**, so a current client will be refused with a
> protocol-mismatch error when it joins it. Until the image is republished, build
> from source (path **B**) so the server and clients match. The image you run is
> built for one protocol version only; clients and server must be on the same one.

### B. Build from source *(developers)*

Compiles the server on your machine — needs the build toolchain and the source tree.

**Prerequisites:** a Linux host with **Docker**, the **Rust toolchain** (stable),
**`trunk`** (`cargo install trunk`), and a checkout of the source tree.

```bash
cd tools/dedicated-server
./build.sh                  # cargo + trunk + docker compose -f docker-compose.build.yml build
docker compose up -d        # runs the locally-built image (no pull)
```

`build.sh` tags the images with the same GHCR names the pull file expects, so the
plain `docker compose up -d` finds and runs your local build. Re-run `./build.sh`
after any engine change.

> **Why two files?** `docker-compose.yml` pulls the published image (path A);
> `docker-compose.build.yml` builds it from source (path B). The publish workflow
> uses the build file, then pushes the result to GHCR — so end users get path A.

---

## How players join

- **Web:** not supported. The browser build is an anonymous, offline sandbox with
  no multiplayer; only the native app joins servers.
- **Native:** in the lobby choose **Join Game** and enter `ws://BOX:6767` (the Join
  dialog accepts `ws://` / `wss://` URLs as well as the legacy `ip:port` form).

> **Plain `ws://` is unencrypted.** The quick-start `ws://BOX:6767` socket carries
> everything in clear text, including the sign-in auth event a signed-in player
> sends when joining. On a trusted home LAN that is a small risk; over the internet
> put the game socket behind TLS (the Caddy front on `:8443` gives you `wss://`, or
> terminate TLS on your own reverse proxy) and have players join with `wss://`.

On a no-domain box the Caddy front uses a self-signed certificate, so a `wss://`
join needs that certificate trusted on the player's machine; use a real domain
(see "Deploying on a VPS") to avoid that.

---

## Configuration

Set these as environment variables in `docker-compose.yml`:

| Var | Default | Meaning |
|-----|---------|---------|
| `AXENSTAX_WORLD` | `server-world` | World folder name (under the volume) |
| `AXENSTAX_GAMEMODE` | `survival` | `survival` \| `creative` \| `adventure` |
| `AXENSTAX_MAX_PLAYERS` | `8` | Max concurrent remote players |
| `AXENSTAX_SERVER_NAME` | `Axe'n'Stax Server` | Display name |
| `AXENSTAX_AUTOSAVE_SECS` | `60` | Autosave interval |
| `AXENSTAX_SEED` | _(random)_ | Fixed terrain seed for a **new** world |

Worlds persist on the `axenstax-worlds` volume and are saved on autosave and on a
graceful `docker stop`.

---

## Verified operator identity (optional)

A server can carry a **verifiable operator identity** rooted in a NIP-46 signer
(Heartwood recommended; Signet / Amber / nsec.app also work). The server holds a
disposable runtime key; your signer signs a one-time, time-bounded **attestation**
over it, and players verify the chain `server key → attestation → your npub`. Your
real key never touches the box. This is **optional and additive** — without it the
server runs anonymously.

```bash
# Pair once (interactive — approve on your signer):
docker compose run --rm axenstax-server axenstax-engine --pair-server

# Print the connect-string to share (carries your npub):
docker compose run --rm axenstax-server axenstax-engine --show-connect
#  axenstax://<host>:6767#op=npub1...

# Renew before the delegation expires (silent — reuses the stored session):
docker compose run --rm axenstax-server axenstax-engine --refresh-delegation
```

| Var | Default | Meaning |
|-----|---------|---------|
| `AXENSTAX_REQUIRE_VERIFIED` | `0` | `1` ⇒ refuse to start without a valid, non-expired attestation |
| `AXENSTAX_IDENTITY_DIR` | `<worlds>/.identity` | where the runtime key + attestation + session live |
| `AXENSTAX_PAIR_RELAY` | `wss://relay.damus.io` | public relay used for the NIP-46 pairing round-trip (`--pair-relay` on the CLI); also the default `AXENSTAX_ADMIN_RELAY` |
| `AXENSTAX_PUBLIC_HOST` | _(unset)_ | `host:port` advertised in the connect-string |
| `AXENSTAX_DELEGATION_DAYS` | `90` | validity window minted at pairing time |

A bare `ws://host:6767` join stays anonymous; a connect-string with `#op=npub…`
makes the client verify the server against that operator. (Attestation kind `30420`
is provisional, pending registration in `forgesworn/nips`.)

---

## Access control (optional)

Two operator-controlled gates on **who** may join (orthogonal to the server's own
identity above):

| Var | Default | Meaning |
|-----|---------|---------|
| `AXENSTAX_REQUIRE_SIGNIN` | `0` | `1` ⇒ only players with a verified Signet identity may join (no anonymous guests) |
| `AXENSTAX_WHITELIST` | _(unset)_ | Comma-separated npubs allowed to join. A non-empty allowlist **implies** sign-in |

The allowlist can also live in `<identity-dir>/whitelist.txt` (one npub per line,
`#` comments allowed) — which is what the **[Operator Console](operator-console.md)**
edits for you, no shell required. Env and file entries are merged.

**Age gates are not yet enforced** — that needs a verifiable Signet *age* credential
(the engine today only carries the kind-31000 display-name credential).

---

## Manage it without a shell — the Operator Console

Rather than editing env and files, run the **[Operator Console](operator-console.md)**
— a small web app where you sign in with your own npub and manage the allowlist /
blocklist, require-sign-in, kicks, server name + max players, and privacy. It ships
in the same `docker-compose.yml` and Caddy routes it at `/admin`.

---

## Deploying on a VPS (real domain — no cert warning)

Point a DNS A/AAAA record at the VPS, set `AXENSTAX_DOMAIN`, and expose 80 + 443.
Caddy then fetches a real Let's Encrypt certificate — players just open
`https://<domain>` with no warning, and the `wss` game socket rides the same cert.

In `docker-compose.yml`: uncomment the `80:80` / `443:443` ports and set
`AXENSTAX_DOMAIN: "play.example.com"`, then `./build.sh && docker compose up -d`.

---

## Deploying on a NAS

- **x86-64 NAS** (Synology Plus/XS, most QNAP x86): works exactly as the quick
  start — build the image on the NAS or any amd64 host, and run it.
- **arm64 NAS** (Pi-based, some Synology/QNAP ARM): the image is arch-aware (Caddy
  is fetched per architecture), but the engine binary is host-built and copied in,
  so it must be **built for arm64** — run `build.sh` on the arm64 box, or
  cross-build the `--server` binary for `aarch64-unknown-linux-gnu` first. The
  forthcoming published image (path A) removes this by building both arches in CI.

---

## Known limitations (alpha)

- **Guest identity only.** Joiners are anonymous unless a server requires sign-in;
  display names are otherwise client-asserted.
- **Native joiners only.** The web build does not join servers.
- **Not the full simulation yet.** The dedicated server currently does not tick
  pistons, hoppers, kegs, dispensers or crops, and inventory and combat are not yet
  server-authoritative. See `tools/dedicated-server/README.md`.
- This is a single self-hostable world server. AxeNStax runs no hosted fleet for
  anyone (the old Spec 07 Agones design is retired).

Design spec: `docs/superpowers/specs/2026-06-16-dedicated-docker-server-design.md`.
Image-publish (one-command install) spec:
`docs/superpowers/specs/2026-06-18-dedicated-server-image-publish-design.md`.
