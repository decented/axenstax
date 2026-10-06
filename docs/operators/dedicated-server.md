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
| **Signed-in players by default** | Only players with a verified Signet identity may join unless you open it to guests; plus allowlist / blocklist by npub — managed by env, file, or the [Operator Console](operator-console.md) |
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

### Tell the server its public address (recommended)

A signed-in player's join is signed for the address they typed (since protocol
v66): `play.example.org`, `203.0.113.7:6767` and so on. If the server knows its own
public address, it refuses a join signed for any other one. That stops a hostile
server from passing a player's sign-in on to yours and joining **as them** (it
could otherwise forward the join, because `ws://` / `wss://` has no end-to-end
binding — the TLS ends at Caddy).

| Set | Example | Notes |
|-----|---------|-------|
| `AXENSTAX_PUBLIC_HOST` (or `--public-host`, repeatable) | `play.example.org` or `203.0.113.7:6767` | Comma-separated public host names or IPs, each optionally `:port`. The first is also the one advertised in the connect-string. Use public addresses (see the LAN caveat below) |
| `AXENSTAX_DOMAIN` | `play.example.com` | Already set on a VPS with a real domain; it is added to the list automatically |

- **An entry without a port covers only the default ports, your own WebSocket
  port and 8443**: `wss://host/ws` through Caddy (443), plain `ws://host` (80),
  `ws://host:6767` (the server's `--port` / `AXENSTAX_WS_PORT`) and
  `wss://host:8443/ws` (the Caddy front on `:8443`). A join on any other port of
  that name is refused, so a hostile server listening on another port of your
  name cannot pass. If you really serve on another port (a different Docker host
  port mapping, your own reverse proxy), list it: `play.example.org:9000`.
- **An entry with a port covers only that port.** `:443` and `:80` also cover a
  join that left the port out.
- **The scheme is not part of what is signed.** A player's signature names the
  host and, unless it is the scheme's default, the port; it does not say `ws` or
  `wss`. So a `:443` entry (or a port-less one) also admits a plaintext
  `ws://host` join on port 80, and a `:80` entry also admits `wss://host` on port
  443. If you want TLS only, enforce it at your proxy (close port 80), not here.
- **List every address players use.** A player who joins by an address that is
  not on the list is refused with a generic *this server expects to be reached at
  its public address*. The refusal deliberately does not say which addresses the
  server is configured with (anyone who can reach the port could read it); the
  server's own log does, as `Rejecting JoinRequest … joiner dialled '…', which is
  not one of this server's public hosts (…)`.
- **LAN addresses are not relay-protected.** If your LAN players join by the
  box's local IP, you can add it as another entry and LAN joins work (the
  dedicated server is WebSocket-only). But an address that is not unique to your
  server (RFC 1918 `10/8`, `172.16/12`, `192.168/16`; CGNAT `100.64/10`;
  loopback; link-local `169.254/16`, `fe80::/10`; IPv6 ULA `fc00::/7`; names
  ending `.local`, `.lan`, `.home.arpa`; single-label names such as `myserver`)
  is one a hostile machine on a player's own network can also hold. It gets that
  player to sign a join for that address and replays it to your public endpoint,
  where your list admits it. The server still boots and still accepts them, but
  the start-up log warns once per entry: `not relay-protected: 192.168.1.20 is
  not unique to this server`. Relay protection is only as strong as your
  public-address entries, so prefer a public domain or IP and have players use it;
  the trade-off is that players on the LAN then join through it too.
- **Check:** the start-up log says `public host: 'play.example.org' …` (and a
  `not relay-protected: …` warning for any LAN-style entry). With
  nothing set it warns instead: `WebSocket joins are not relay-protected: set
  --public-host …` — the server then accepts a join signed for any address, as
  before.
- A malformed entry (a scheme, a path, a bad port) stops the server at start-up
  with the reason.
- Operator tools still need a direct (QUIC) connection, whatever you set here.

> **Upgrading from an engine before protocol v66?** If you already set
> `AXENSTAX_PUBLIC_HOST`, `--public-host` or `AXENSTAX_DOMAIN`, the check is now on:
> players who join by a different address (for example the LAN IP of a box that
> also has a domain) are refused until you add that address to
> `AXENSTAX_PUBLIC_HOST` (it will be accepted, with the not-relay-protected
> warning above). **If your existing value carries a port** (older docs said
> `host:port`), drop the port or add the bare host as a second entry: a
> `host:6767` entry admits only `ws://host:6767`, so browsers coming through
> Caddy (`wss://host/ws`, `wss://host:8443/ws`) would be refused. **If you serve
> on a port that is neither 80, 443, 8443 nor your `--port`**, list it explicitly
> (`play.example.org:9000`): a port-less entry no longer means "any port".
> Players also need a v66 game build to join.

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
| `AXENSTAX_SIM_DISTANCE` (or `--sim-distance`) | `8` | Radius, in 16-block columns, the server keeps loaded and simulated around each connected player and the world spawn (2–16). Higher = more RAM and CPU per player |
| `AXENSTAX_SEED` | _(random)_ | Fixed terrain seed for a **new** world |

Worlds persist on the `axenstax-worlds` volume and are saved on autosave and on a
graceful `docker stop`.

**Simulation distance.** The server loads the world around every connected player
(and around spawn) as they move, generating new ground or reloading saved ground
a couple of columns per tick, and lets go of areas nobody is near. Built or dug
areas are never lost when they unload: they stay in memory and are written on
the next autosave. The default radius of 8 columns (128 blocks) suits a small
server; each extra column of radius costs memory and generation time per player
who wanders off alone.

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
| `AXENSTAX_PUBLIC_HOST` | _(unset)_ | This server's public host name(s) or IP(s), comma-separated (`--public-host`, repeatable). The first is advertised in the connect-string as `axenstax://<host>:<ws port>`; all of them are the addresses joins must be signed for (see "Tell the server its public address") |
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
| `AXENSTAX_ALLOW_GUESTS` (or `--allow-guests`) | `0` | `1` ⇒ also admit anonymous guests. Unset, **only players with a verified Signet identity may join**. The flag may stand alone (`--allow-guests`) or take a value (`--allow-guests 1`, `--allow-guests=0`; `1`/`true`/`yes`/`on` or `0`/`false`/`no`/`off`) — a falsy value keeps sign-in required, and any other value refuses to start |
| `AXENSTAX_WHITELIST` | _(unset)_ | Comma-separated npubs allowed to join. A non-empty allowlist **implies** sign-in |

**Sign-in is required by default** (since 2026-10-06). The old
`AXENSTAX_REQUIRE_SIGNIN` / `--require-signin` switch is no longer read — it is
still accepted (and logged) so existing scripts start, but it changes nothing. To
let guests in, set `AXENSTAX_ALLOW_GUESTS=1` or start with `--allow-guests`. The
Operator Console's *require sign-in* toggle (and the `require-signin` admin
command) writes `<identity-dir>/require_signin`, which overrides both and survives
restarts.

**Upgraded from before 2026-10-06? Check your box.** The old setup wizard's "Who can
come in?" step defaulted to *Anyone*, and it saved that choice as a
`<identity-dir>/require_signin` file reading `false` (`<identity-dir>` is
`<worlds>/.identity`, i.e. `/worlds/.identity` in the Docker setup, unless
`AXENSTAX_IDENTITY_DIR` says otherwise). That file beats the new default, so a
server set up that way **keeps admitting guests** after the engine upgrade until you
change it. The engine reads only that file: `true` = sign-in required, anything else
(including an empty file) = guests admitted.

- **Check:** the start-up log line `access :` says `sign-in required` or `guests
  admitted`; the Operator Console shows the same in *Access → Require sign-in*; or
  `cat /worlds/.identity/require_signin`.
- **Switch to sign-in:** tick *Require sign-in* in the Console (or re-run the setup
  wizard and pick *Signed-in players*), or write `true` into the file, or delete it
  (the default then applies — sign-in required, unless the server is started with
  `--allow-guests` / `AXENSTAX_ALLOW_GUESTS=1`). The running server picks it up
  within about 5 seconds; no restart.

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

The domain also becomes the server's public address for join checks (see "Tell
the server its public address"): players who join by the box's IP instead are
refused, so add any other address they use to `AXENSTAX_PUBLIC_HOST` (a LAN
address works but is not relay-protected; see "Tell the server its public
address").

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

- **Signed-in joiners by default.** A player must be signed in (a verified Signet
  identity) unless the server admits guests; a guest's display name is
  client-asserted.
- **Native joiners only.** The web build does not join servers.
- **Not the full simulation yet.** The dedicated server currently does not tick
  pistons, hoppers, kegs, dispensers or crops, and inventory and combat are not yet
  server-authoritative. See `tools/dedicated-server/README.md`.
- This is a single self-hostable world server. AxeNStax runs no hosted fleet for
  anyone (the old Spec 07 Agones design is retired).

Design spec: `docs/superpowers/specs/2026-06-16-dedicated-docker-server-design.md`.
Image-publish (one-command install) spec:
`docs/superpowers/specs/2026-06-18-dedicated-server-image-publish-design.md`.
