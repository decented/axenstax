# Axe'n'Stax Dedicated Server (Docker)

A self-hostable multiplayer server. **Both the web app and the native app join
the same world** — the web client can't speak the native QUIC transport, so the
server speaks **WebSocket**, which both targets use.

One container runs two things behind one HTTPS origin:

- the **headless game server** (`axenstax-engine --server`) — a headless,
  GPU-free world server that currently simulates mobs, carts and
  persistence, among other things. It is **not yet the full authoritative simulation**: it does
  **not currently tick pistons, hoppers, kegs, dispensers or crops**, and
  inventory and combat are **not yet server-authoritative** (a fix for the block
  ticks is in progress);
- a **Caddy front** that serves the web client and reverse-proxies the game
  socket, so the self-signed cert is trusted once and reused for `wss`.

```
                       ┌──────────── one container ────────────┐
 https://BOX:8443  ───▶│ Caddy :8443  /      → web client (PWA) │
                       │              /ws     → 127.0.0.1:6767   │
 ws://BOX:6767     ───────────────────────────▶ engine --server │
                       │                         /worlds (volume)│
                       └───────────────────────────────────────┘
```

> **Security note.** The plain `ws://BOX:6767` quick-start socket is unencrypted:
> the sign-in auth event a player sends when joining travels in clear text unless
> you front the socket with TLS (the bundled Caddy `wss://` route on `:8443`, or
> your own reverse proxy). Use `wss://` anywhere beyond a trusted LAN.

> **Protocol version.** The published `latest` image predates protocol v64 (the
> current `PROTOCOL_VERSION`). Build from source (`./build.sh`) until it is
> republished.

## Quick start

There are two compose files:

- **`docker-compose.yml`** — the END-USER file: pulls prebuilt images from GHCR.
  Only Docker required, no source/toolchain.
- **`docker-compose.build.yml`** — the developer/CI file: builds the images from
  source (host-compiled binary + bundle via `build.sh`), tagging them with the same
  GHCR names so the pull file reuses them.

```bash
# A. Run a published image (end users) — needs only Docker:
cd tools/dedicated-server
docker compose up -d              # pulls ghcr.io/decented/axenstax-{server,operator-console}

# B. Build from source (developers) — needs Rust + trunk on the host:
./build.sh                        # cargo + trunk + docker compose -f docker-compose.build.yml build
docker compose up -d              # finds the locally-built image, runs it (no pull)

# Then, from any machine on the same network:
#    Web    →  open  https://<this-box-ip>:8443   (accept the cert warning once)
#    Native →  Join  ws://<this-box-ip>:6767
```

Find `<this-box-ip>` with `ip -4 addr` / `hostname -I` (e.g. `192.168.1.20`).

> The published images are **public on GHCR** (`ghcr.io/decented/axenstax-server` +
> `…-operator-console`), so path **A** needs only Docker. They're rebuilt by
> `.github/workflows/publish-server-image.yml` (manual dispatch). Path **B** is only
> needed to build from source.

## First-run setup wizard

On a **fresh box** the server doesn't jump straight into a default world. Instead it
comes up in **setup mode**: Caddy and the Operator Console (`/admin`) are live, but the
game engine **waits** until you finish a short setup wizard. This lets your first choice
— *what kind of place is this?* (**Gallery / Creative / Survival / Adventure**) — apply
to the very first world, with no throwaway default to undo.

1. `docker compose up -d`, then open **`https://<box>:8443/admin`**.
2. Sign in with your operator npub (signet-login / mySignet — see *Operator Console
   access* below). The owner npub is the one established when the box was provisioned.
3. The wizard walks you through a few simple questions (kind of place, name, who can
   join, a couple of toggles) and presses **Create**. The engine then boots into the
   world you chose.

The wizard is **re-runnable** any time from the console (Identity panel → *Re-run setup
wizard*). Re-running and changing the kind of place starts a **fresh** world; the old
one is **archived** (renamed `…​.archived-<timestamp>` on the volume), never deleted.

Skipping is possible (a small link) but discouraged — the box then boots on the compose
defaults and the dashboard keeps a "finish setup" reminder. To bypass the gate entirely
(headless / CI), set **`AXENSTAX_SKIP_WIZARD=1`**.

Design: `docs/superpowers/specs/2026-06-21-server-setup-wizard-design.md`.

## How clients join

> **Sign-in is required by default** (since 2026-10-06): only players with a
> verified Signet identity may join. Start the server with `--allow-guests`
> (or set `AXENSTAX_ALLOW_GUESTS=1`) to admit anonymous guests too.

- **Web (Chromium PWA):** opening `https://BOX:8443` boots a guest and
  auto-joins this server (the page sets `window.AXENSTAX_DEDICATED_WS`). The
  web build has no sign-in, so browser players can only join a server that
  admits guests (`AXENSTAX_ALLOW_GUESTS=1`); otherwise they are turned away
  with "sign-in required".
- **Native:** in the lobby choose **Join Game** and enter `ws://BOX:6767`
  (the native Join dialog accepts `ws://`/`wss://` URLs as well as the legacy
  `ip:port` QUIC form).

## Configuration (env in `docker-compose.yml`)

| Var | Default | Meaning |
|-----|---------|---------|
| `AXENSTAX_WORLD` | `server-world` | World folder name (under the volume) |
| `AXENSTAX_GAMEMODE` | `survival` | `survival` \| `creative` \| `adventure` |
| `AXENSTAX_MAX_PLAYERS` | `8` | Max concurrent remote players |
| `AXENSTAX_SERVER_NAME` | `Axe'n'Stax Server` | Display name |
| `AXENSTAX_WS_PORT` | `6767` | Native WebSocket game port ("STAX"/"6-7") |
| `AXENSTAX_AUTOSAVE_SECS` | `60` | Autosave interval |
| `AXENSTAX_SEED` | _(random)_ | Fixed terrain seed for a **new** world |
| `AXENSTAX_SKIP_WIZARD` | `0` | `1` ⇒ skip the first-run setup gate; boot immediately on these defaults |

> Note: with the first-run wizard active (default), the values above are **starting
> defaults** — the setup wizard (and the console) override most of them
> (game mode, name, access, showcase) via `.identity/server.env`, which the engine
> sources at boot.

Worlds persist on the `axenstax-worlds` volume and are saved on autosave + on
graceful `docker stop`.

## Verified operator identity (optional)

A server can carry a **verifiable operator identity** rooted in a NIP-46 **bunker**
signer (**Amber, nsec.app, or Heartwood**). The server holds a disposable runtime
key; your signer signs a one-time, time-bounded **attestation** over it, and players
verify the chain `server key → attestation → your npub`. Your real key never touches
the box. This is **optional and additive** — without it the server runs anonymously
exactly as above.

Pair once (interactive — approve in your signer):

```bash
docker compose run --rm --entrypoint axenstax-engine axenstax-server --pair-server
# Scan the printed QR (or paste the nostrconnect:// URI) into your signer,
# approve, and the signed attestation is stored on the worlds volume.
```

> ⚠️ **Two gotchas:**
> 1. The `--entrypoint axenstax-engine` override is **required** — the image
>    entrypoint ignores extra args and would otherwise just boot a *second* server
>    on your world volume instead of pairing.
> 2. Your signer must be a NIP-46 **bunker** that can *accept* a pasted/scanned
>    `nostrconnect://` link. **mySignet.app cannot do this pairing** — it's a
>    redirect/sign-in signer with no "connect an app via nostrconnect" entry point
>    (tracked upstream, not yet fixed). Use **Amber / nsec.app / Heartwood**.
>    If you only want to gate the **Operator Console** (`/admin`), you don't need a
>    bunker at all — see "Operator Console access" below.

Then print the connect-string to share with players (carries your npub):

```bash
docker compose run --rm --entrypoint axenstax-engine axenstax-server --show-connect
#  axenstax://<host>:6767#op=npub1...
```

Renew before the delegation expires (silent — reuses the stored session):

```bash
docker compose run --rm --entrypoint axenstax-engine axenstax-server --refresh-delegation
```

Identity configuration (env in `docker-compose.yml`):

| Var | Default | Meaning |
|-----|---------|---------|
| `AXENSTAX_REQUIRE_VERIFIED` | `0` | `1` ⇒ refuse to start without a valid, non-expired attestation |
| `AXENSTAX_IDENTITY_DIR` | `<worlds>/.identity` | where the runtime key + attestation + session live |
| `AXENSTAX_PAIR_RELAY` | `wss://relay.damus.io` | public relay used for the NIP-46 pairing round-trip (`--pair-relay` on the CLI); also the default `AXENSTAX_ADMIN_RELAY` |
| `AXENSTAX_PUBLIC_HOST` | _(unset)_ | `host:port` advertised in the connect-string |
| `AXENSTAX_DELEGATION_DAYS` | `90` | validity window minted at pairing time |

A bare `ws://host:6767` join stays anonymous; a connect-string with `#op=npub…`
makes the client verify the server against that operator. Attestation event kind
`30420` is provisional (pending registration in `forgesworn/nips`).

### Operator Console access (`/admin`)

The web Operator Console at `https://<host>/admin` is gated to **operator npubs**.
You sign in there with **normal signet-login** (mySignet works fine for *this* — it's
the redirect flow, not nostrconnect), and the console admits you if your signed-in
npub is an operator. **It matches on the npub only — no signature.** So you have two
ways to be an operator:

- **Primary operator** — the npub from the attestation above (set by `--pair-server`).
- **Additional operators (co-admins)** — listed in **`<identity-dir>/operators.txt`**,
  one `npub…` (or 64-char hex) per line; `#` comments (full-line *and* inline) ignored.
  The console reads it **live** (no restart). This lets you grant `/admin` to more than
  one person without re-pairing:

  ```bash
  # add a co-admin (the file lives on the worlds volume, so it persists):
  docker exec axenstax-server sh -c \
    'mkdir -p /worlds/.identity && echo "npub1co-admin…  # name" >> /worlds/.identity/operators.txt'
  ```

> **Self-hoster shortcut:** because the gate is npub-only, you can run a console-only
> server with **no bunker pairing at all** — just drop an `attestation.json` containing
> `{"pubkey":"<your-npub-hex>", …}` (or add yourself to `operators.txt`) into
> `<identity-dir>`. A first-class `AXENSTAX_OPERATOR_NPUB` flag for this is on the
> roadmap; the attestation chain (above) is only needed when *players* must
> cryptographically verify the server's operator.

## Access control (optional)

Two operator-controlled gates on **who** may join (orthogonal to the server's own
identity above):

| Var | Default | Meaning |
|-----|---------|---------|
| `AXENSTAX_ALLOW_GUESTS` (or `--allow-guests`) | `0` | `1` ⇒ also admit anonymous guests. Unset, **only players with a verified Signet identity may join** |
| `AXENSTAX_WHITELIST` | _(unset)_ | Comma-separated npubs allowed to join. A non-empty allowlist **implies** sign-in |

The sign-in requirement is **on by default**. `AXENSTAX_REQUIRE_SIGNIN` and
`--require-signin` are no longer read (they are accepted, and logged, so old
scripts still start); use `AXENSTAX_ALLOW_GUESTS` / `--allow-guests` to open the
server to guests. A `<identity-dir>/require_signin` file — written by the
`require-signin` admin command or the Operator Console's toggle — overrides both,
so a box that was opened to guests from the console stays open after a restart.

The allowlist can also live in `<identity-dir>/whitelist.txt` — one npub per line,
`#` comments allowed — which is easier to edit and is what runtime admin commands
will append to (later track). Env and file entries are merged.

```bash
# Invite-only server: only these two npubs may join.
AXENSTAX_WHITELIST="npub1abc…,npub1def…" docker compose up -d
```

**Age gates are not yet enforced** — that needs a verifiable Signet *age* credential
(the engine today only carries the kind-31000 display-name credential). When Signet
ships an age attestation, the per-server minimum-age gate slots in alongside the
allowlist here.

## Creator Gallery / showcase kiosk (optional)

Turn this box into a contained, unattended **gallery / booth**: a guest opening
`:8443` is locked to read-only **Adventure**, walks a curated room, **clicks
exhibits to collect** them into a personal basket, and **exits to one terminal
screen** (no escape to the lobby). Off by default — a normal server is unaffected.
Kiosk visitors are browser guests, so a kiosk box also needs
`AXENSTAX_ALLOW_GUESTS=1` (sign-in is required by default).

**1 — Author the gallery (native, on your own machine).** In a creative world,
drop your images into `worlds/<name>/exhibits/` and place them with the
[`/exhibit`](../../docs/spec/05-gameplay-systems.md) command (wall art + standing
billboards; `place`/`move`/`resize`/`yaw`/`image`/`label`/`delete`).

**2 — Put the world on the box.** Copy the whole world folder — both `world.dat`
(carries the exhibit *placements*) **and** the `exhibits/` image files — onto the
worlds volume as the served world:

```bash
docker cp worlds/<name>/.  axenstax-server:/worlds/server-world/
# (or write directly into the `dedicated-server_axenstax-worlds` volume)
```

> The exhibit **placements** travel automatically (they live in `world.dat`); the
> **image files** must be on the volume too — they are served from
> `/worlds/<world>/exhibits/` by the `/exhibits/*` Caddy route. *(Packing images
> into a single portable `.axeworld` is a deferred convenience — folder-copy is the
> supported path today; see `docs/superpowers/specs/2026-06-19-creator-gallery-showcase-design.md` §7.)*

**3 — Arm the kiosk** (uncomment in `docker-compose.yml`):

```yaml
AXENSTAX_SHOWCASE: "1"
# AXENSTAX_AUTO_LOOP_SECS: "60"   # optional: reset to a fresh session for a booth
```

Then `docker compose up -d`. A web client renders the exhibits (defs over the
join protocol, images over `/exhibits/`); the kiosk loop contains the session. A
gallery with **no** exhibits is the **2a demo-kiosk** (containment + exit screen)
— a valid events booth on its own.

## Operator admin (runtime control)

Change the policy on a **running** server without restarting it, authenticated by
the **same Heartwood key** that signed the server's identity — no shared password.
The operator signs a command event (kind `27422`) carrying one `["cmd", …]` tag
plus a `["server","npub1…"]` tag naming **this server's runtime key** (the
audience — `--admin-sign` adds it automatically from the server's identity dir;
a hand-built command must include it):

| Command tag | Effect |
|-------------|--------|
| `["cmd","whitelist-add","npub1…"]` | add an npub to the allowlist |
| `["cmd","whitelist-remove","npub1…"]` | remove an npub |
| `["cmd","require-signin","true"\|"false"]` | toggle the sign-in requirement |

Apply a signed command (the server verifies it was signed by *its* operator and is
fresh, then writes the change to the on-disk policy):

```bash
docker compose run --rm --entrypoint axenstax-engine axenstax-server --admin /path/to/command.json
```

The running server re-reads its policy every ~5 s, so the change goes live within
seconds. Commands older than 5 minutes, signed by anyone other than the
operator, missing the `server` tag or addressed to another server, or already
applied once (event ids are remembered for the 5-minute window in
`admin-seen.json` beside the identity), are rejected. (Delivering commands to the box over a relay — true
remote, no shell — is a later owner-side step; the verify+apply path is in place.)

## The self-signed certificate

There's no public domain, so the front uses a self-signed cert. Browsers show a
one-time warning — click through (**Advanced → Proceed**). Because the page and
the game socket share the one origin, the `wss` connection reuses that trust with
no second prompt. (WebGPU also *requires* a secure context off-localhost, which
is why the web path is HTTPS, not plain HTTP.)

## Deploying on a VPS (real domain — no cert warning)

Point a DNS A/AAAA record at the VPS, set `AXENSTAX_DOMAIN`, and expose 80 + 443.
Caddy then fetches a real Let's Encrypt certificate — players just open
`https://<domain>` with no warning, and the `wss` game socket rides the same cert.

In `docker-compose.yml`: uncomment the `80:80` / `443:443` ports and set
`AXENSTAX_DOMAIN: "play.example.com"`, then `./build.sh && docker compose up -d`.
(The self-signed `:8443` front is used only when `AXENSTAX_DOMAIN` is unset.)

## Deploying on a NAS

- **x86-64 NAS** (Synology Plus/XS, most QNAP x86): works exactly as the quick
  start — build the image on the NAS, or on any amd64 host, and run it.
- **arm64 NAS** (Pi-based, some Synology/QNAP ARM): the image is arch-aware (Caddy
  is fetched per `TARGETARCH`), but the engine binary is host-built and copied in
  (`build.sh`), so it must be **built for arm64**. Either run `build.sh` on the
  arm64 NAS itself, or cross-build the `--server` binary for
  `aarch64-unknown-linux-gnu` and stage it before `docker compose build`.
- A fully self-contained, cross-arch image (compile the engine inside Docker via
  `docker buildx --platform linux/amd64,linux/arm64`) is the next step — it removes
  the host-toolchain requirement entirely. Not yet wired here (the WASM/trunk web
  bundle build inside Docker needs verifying on real hardware).

## Known limitations (alpha)

- **Web joiners are guests.** The browser build has no sign-in, so a server
  serving web players must set `AXENSTAX_ALLOW_GUESTS=1`. Native clients that
  are signed in join with their verified Signet identity; a guest's display
  name is client-asserted.
- **Web-player edits not yet propagated.** A browser joiner sees the shared world
  and everyone moving, but its own block edits aren't sent to the server yet
  (the ~30 edit hooks are still native-gated). Native joiners' edits propagate.
- Not the Spec 07 Agones fleet — this is a single self-hostable world server.

See the design spec: `docs/superpowers/specs/2026-06-16-dedicated-docker-server-design.md`.
