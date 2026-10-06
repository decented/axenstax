# Operator Console — run + use your server's web console

The **Operator Console** is a small web app for running an Axe'n'Stax dedicated
server. Open it in a browser, sign in with **your own npub**, and manage who can
join, the server's settings + privacy, and see who's been playing — no shell, no
passwords.

> **For an AI assistant:** this page is written so you can stand the console up for
> a user and explain it. Everything here is exact. The console is the
> `tools/sites/console/` FastAPI service; it ships with the dedicated server.

---

## What it does

| | |
|---|---|
| **Access** | Allowlist + blocklist (by npub), require-sign-in toggle, kick a player |
| **Server** | Name, blurb, region, max players, "announce" (publish a discoverable Server Card) |
| **Privacy** | Tracking level (default **none**), retention, and erasure (forget one player / purge all history) |
| **See** | Your operator identity + delegation expiry, and operator-private session history |

It is a **management surface only** — it edits the same policy files your server
already reads, and the server picks up changes within ~5 seconds. It never
touches the game world or player data beyond the operator-private session log
(npub + join/leave times; no IP, no location).

---

## What it does NOT do (be honest with players)

The console proves the server **operator's identity** cryptographically (only the
operator key can sign in). It **cannot prove the server's behaviour** — e.g. a
"no tracking" setting is a **signed declaration**, not a technical guarantee.
The honest line, which the UI keeps: **identity is _verified_; the privacy posture
is _declared_.** The open-source default build genuinely honours "no tracking";
trusting a modified build is a reputation matter.

It also does not set the server's **public address** (the relay protection for
WebSocket joins, protocol v66) — neither the setup wizard nor the dashboard has
that field yet. Set `AXENSTAX_PUBLIC_HOST` (and/or `AXENSTAX_DOMAIN`) in
`docker-compose.yml`; see "Tell the server its public address" in the
[dedicated server guide](dedicated-server.md).

---

## Prerequisites

1. A **dedicated server** running (Docker — see the Dedicated Server guide).
2. The server has an **operator identity** (it's been *paired*). On the server box:
   ```bash
   axenstax-engine --pair-server
   ```
   This creates `<worlds>/.identity/attestation.json`. Until a server is paired,
   the console shows a "pair first" message and nobody can sign in.
3. You have the **operator's Signet identity** (the npub you paired with) on your
   phone or browser signer.

---

## Install / run

### Production (with the dedicated server) — recommended
The console ships as a service in the dedicated server's `docker-compose.yml`.
Bring the stack up as normal:

```bash
cd tools/dedicated-server
docker compose up -d --build
```

The console comes up automatically and Caddy routes `/admin` to it (sharing the
server's identity volume). Open it at:

```
https://<your-domain-or-box-ip>:8443/admin      # LAN / no-domain (self-signed cert)
https://<your-domain>/admin                       # VPS with AXENSTAX_DOMAIN set
```

Sign in with the operator npub. Done.

### Local dev (no Docker)
```bash
cd tools/sites/console
AXENSTAX_IDENTITY_DIR=/path/to/your/.identity ./start.sh
# → http://localhost:8101
```

---

## Using it

1. **Sign in.** The login page shows the operator npub this server expects. Click
   *Sign in with Signet*, approve on your signer. Only that npub gets in — any
   other identity is refused.
2. **Access** — add npubs to the **allowlist** (only listed npubs may join; blank =
   open) or **blocklist** (refused even if allowlisted — block always wins). Toggle
   *require sign-in* — on by default for a new server; turning it off admits anonymous
   guests and stays off across restarts. (A server set up with the old wizard, whose
   default was "Anyone", already has it off — tick it if you want sign-in; see the
   upgrade note in the [dedicated server guide](dedicated-server.md#access-control-optional).) *Kick* removes a connected player (queued; applied
   within ~5s).
3. **Server** — set the name / blurb / region / max players, and toggle *announce*
   to publish a discoverable Server Card so players can find you by your npub.
4. **Privacy** — choose **None** (keep nothing — the default) or **Sessions** with a
   retention window. *Forget* erases one player's history; *Purge* erases all of it.

Changes are written to the server's identity dir and applied on its next reload.

---

## Security

- **Only the operator can log in.** The console checks the signed-in npub against
  the server's attestation; everyone else is refused (`403`).
- **Serve it over TLS.** In the Docker stack Caddy terminates HTTPS; the session is
  an HMAC cookie. Don't expose the raw `:8101` over the internet without TLS.
- **It only touches policy files** in `<worlds>/.identity/`. The only player data it
  can read is the operator-private session log.

---

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| "This server has no operator identity yet" | Not paired — run `axenstax-engine --pair-server` on the box. |
| Sign-in succeeds but you land back on login | You signed in with a non-operator npub. Use the key the server was paired with (shown on the login page). |
| `/admin` 404s | The console service isn't up, or Caddy didn't get the route — `docker compose up -d --build` and check `docker compose logs axenstax-console`. |
| Changes don't take effect | The server applies policy on a ~5s reload — wait a moment. Confirm both services share the `axenstax-worlds` volume. |

---

## How it's built (B-0)

The game engine has no HTTP server (it's a 20-TPS game loop), so the console is a
**separate FastAPI sidecar** rather than HTTP-in-engine. It manages the on-disk
policy files the engine already reloads — zero engine change. Design + rationale:
`docs/superpowers/specs/2026-06-18-operator-console-sidecar.md`. Code +
configuration: `tools/sites/console/`.
