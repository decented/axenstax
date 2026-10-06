# Axe'n'Stax — Operator Console (web sidecar)

The web management surface for a dedicated-server operator. Manage who can join,
the server's settings + privacy posture, and see session history — from a browser,
signed in with your own operator npub.

**Why a sidecar (B-0):** the game engine has no HTTP server (it's a 20-TPS game
loop). Rather than bolt axum into it, the console is a small FastAPI service that
manages the **same on-disk policy files the engine already reloads every ~5s**.
Zero engine change. Design: `docs/superpowers/specs/2026-06-18-operator-console-sidecar.md`.

## What it does

- **Setup wizard** — a WordPress/OS-style first-run flow. On first sign-in the operator
  picks *what kind of place this is* (Gallery / Creative / Survival / Adventure), a name,
  who can join, and a couple of toggles — in a few super-simple steps an 8-year-old could
  do with a parent. Re-runnable any time (Identity panel → *Re-run setup wizard*). Writes
  the same files below **plus** `server.json` (its own record), `server.env` (boot config
  the dedicated-server entrypoint sources) and the `setup-complete` gate marker. Module:
  `wizard.py`; design `docs/superpowers/specs/2026-06-21-server-setup-wizard-design.md`.
- **Identity** — your operator npub, the runtime key, delegation expiry.
- **Access** — allowlist + blocklist (npubs), require-sign-in toggle (on by default — the
  engine requires sign-in unless the `require_signin` file says anything but `true` or it was
  started with `--allow-guests`; a box set up with the old wizard, whose default was
  "Anyone", already has that file saying `false` and stays guest-open until you tick the
  toggle), kick a player.
- **Server** — name / about / region / max players / announce (publish a Server Card).
- **Privacy** — tracking level (`none` default / `sessions:<days>`), retention, and the
  erasure controls (forget one player / purge all history). Identity is *verified*;
  the privacy posture is *declared* (see the operator docs).
- **Session history** — operator-private (npub + timestamps only; no IP/geo).

## How it works

```
browser ──HTTPS──> Caddy (/admin*) ──> this sidecar (:8101) ──reads+writes──> <worlds>/.identity/
                                                                                  ▲ engine reloads ~5s
```

It reads/writes (`identity.py`): `attestation.json` (→ operator npub), `whitelist.txt`,
`blocklist.txt`, `require_signin`, `console.json`, `sessions.jsonl`, `kick`.

**Auth:** you sign in with **your own npub** via signet-login (same flow as the game
site). Only the npub the server's `attestation.json` chains to may log in — every
other identity is refused (`app.py::current_operator_npub`). No passwords.

## Run it

### Production (with the dedicated server)
The dedicated-server Docker compose runs the console as a service and Caddy routes
`/admin` to it — see `tools/dedicated-server/README.md`. Just open
`https://<your-domain>/admin`.

### Local dev
```bash
AXENSTAX_IDENTITY_DIR=/path/to/.identity ./start.sh   # http://localhost:8101
```

## Config (env)

| Var | Default | Meaning |
|---|---|---|
| `PORT` | `8101` | listen port |
| `AXENSTAX_IDENTITY_DIR` | `$AXENSTAX_WORLDS_DIR/.identity` | the server's identity dir |
| `AXENSTAX_WORLDS_DIR` | `/worlds` | worlds root (used if `AXENSTAX_IDENTITY_DIR` unset) |
| `CONSOLE_DATA_DIR` | `<identity-dir>/console-web` | where the session-HMAC secret lives |

## Security

- Only the operator key can log in (gated on the attestation). Sessions are HMAC
  cookies (90-day sliding, same machinery as the game site).
- The console only ever touches the identity-dir policy files; the only player data
  it can read is the operator-private session log (npub + times).
- Serve it behind Caddy TLS; the `/admin` route is the only exposure.

## Files

| File | Role |
|---|---|
| `app.py` | FastAPI routes + the operator gate |
| `identity.py` | read/write the identity-dir files (bech32 + telemetry; the source of truth) |
| `auth.py` | signet-login challenge/verify + session cookie (from the game site) |
| `templates/{login,dashboard}.html` | the UI |
| `static/console{,-login}.{js,css}` | the browser flow + dashboard actions |
| `Dockerfile` | the console image (used by the dedicated-server compose) |
