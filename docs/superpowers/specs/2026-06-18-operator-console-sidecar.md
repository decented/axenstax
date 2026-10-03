# Operator Console — web sidecar (B-0 resolved)

**Date:** 2026-06-18
**Status:** DECISION + DESIGN, building. Resolves amendment **B-0** from the Operator
Console spec (`2026-06-17-operator-console-design.md` §B-0): the engine has no HTTP
server, so the web console is a **separate FastAPI sidecar**, not axum-in-engine.
**Builds on:** the engine pieces shipped overnight — the identity-dir file model
(`whitelist.txt`, `blocklist.txt`, `console.json`, `sessions.jsonl`, `require_signin`,
`kick`, `attestation.json`) and `console_auth` (the nonce-signing login primitive).

---

## 1. The decision (B-0)

**Sidecar, not axum-in-engine.** Reasons:
- The engine is a 20-TPS game loop; bolting axum/hyper/tower in bloats the binary and
  widens the game server's attack surface for a non-gameplay feature.
- Every other AxeNStax surface is a FastAPI site under Caddy (`tools/sites/*`). The
  dedicated server already runs Caddy + Docker — a console sidecar drops into that
  pattern with one `/admin` route.
- The seams already exist: the console reads/writes the operator's **identity dir**
  (the files the engine reloads every ~5s) and authenticates via the **`console_auth`**
  nonce-signing model. **Zero engine change** — which is the whole point of B-0.

## 2. Architecture

```
  operator's browser ──HTTPS──> Caddy (/admin*) ──> console sidecar (FastAPI :8101)
                                                          │  reads + writes
                                                          ▼
                                          <worlds>/.identity/  (shared volume)
                                          whitelist.txt · blocklist.txt · console.json
                                          require_signin · kick · sessions.jsonl · attestation.json
                                                          ▲  reloads every ~5s
                                                   the engine (game server)
```

- **No new control protocol.** The console edits the same on-disk policy files the
  engine already reloads (the file model from Spec B). Reading those files + the
  attestation gives the dashboard; writing them applies changes within ~5s.
- **Auth = operator-npub signed nonce.** Login issues a nonce; the operator signs a
  kind-27423 console-login event with their npub (browser nostr signer / signet-login);
  the sidecar verifies (signature + `pubkey == operator npub from the attestation` +
  nonce + freshness) using `secp256k1`/`nostr-sdk` — the same crypto the game site
  already uses. A short-lived session cookie follows. No passwords.

## 3. Components (this build)

| Path | Role |
|---|---|
| `tools/sites/console/app.py` | FastAPI routes: login, auth challenge/verify, dashboard, policy POSTs |
| `tools/sites/console/identity.py` | read/write the identity-dir files (the single source of truth) |
| `tools/sites/console/auth.py` | nonce issue + signed-event verify against the operator npub |
| `tools/sites/console/templates/` | `login.html`, `dashboard.html` |
| `tools/sites/console/static/` | css + the browser nonce-signing JS |
| `tools/sites/console/{requirements.txt,start.sh,README.md,Dockerfile,.gitignore}` | run + deploy |
| `tools/dedicated-server/{docker-compose.yml,Caddyfile,Caddyfile.domain}` | add the `console` service + `/admin` route |
| `tools/sites/docs/` (+ template) | the public **explainer page**: what it is / install / use |
| `docs/operators/operator-console.md` | the **AI-facing install + operate guide** |

## 4. What the console does (operator capabilities)

- **See:** server identity (operator npub, runtime key, delegation expiry), the access
  policy (allow/block lists, sign-in requirement), the descriptor + capacity + announce +
  privacy settings, and **session history + aggregates** (operator-private; no live roster
  yet — that's the in-game panel / a future `roster.json`).
- **Manage:** add/remove allowlist + blocklist npubs, toggle sign-in, set cap / name /
  about / region / announce / privacy level, kick a player (queued), and the privacy
  erasure controls (forget a player / purge history).

## 5. Out of scope / follow-ups

- **Live roster** (who's connected *right now*) — needs the engine to write `roster.json`
  or the in-game Operator panel (B-7a). The console shows session *history*.
- **Remote (off-box) console** — this v1 runs on the server box (shared volume). A remote
  console would use the relay-signed `AdminCommand` path instead of file writes.
- **Live deploy verification** — needs a running dedicated server with a paired identity
  (owner boundary).

## 6. Security posture

- Auth is the operator's own key (declared identity model) — only the npub the server's
  attestation chains to can log in.
- The console only ever touches the identity-dir policy files; it cannot read player data
  beyond the operator-private session log (npub + timestamps, no IP/geo — Spec C).
- Runs behind Caddy TLS; the `/admin` route is the only exposure.
