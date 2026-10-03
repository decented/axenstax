# Operator Console — server management & traffic visibility

**Date:** 2026-06-17
**Status:** DESIGN — approved in brainstorm, ready for an implementation plan. Spec
**B** of three (A = Server Card / discovery; B = Operator Console; C = Privacy &
tracking transparency). Build order A → B → C.
**Companion to:** [Server Card (Spec A)](2026-06-17-server-card-design.md) ·
[Heartwood-signed server identity (keystone)](2026-06-17-heartwood-signed-server-identity-design.md) ·
[Server operator/creator identity model](2026-06-17-server-operator-identity-creator-model.md) ·
[Dedicated Docker server](2026-06-16-dedicated-docker-server-design.md)
**Constraints honoured:** `feedback_signet_boundary` (control plane is plain Nostr
+ NIP-46, nothing AxeNStax-specific in Signet), `project_identity_default_nonpublic`
(player presence telemetry stays operator-private, never relayed publicly),
`project_shared_infra_strategy` (the console core is a generic primitive),
`npub-only display rule` (operator-facing surfaces render npub, not hex).

---

## 1. What this adds (and what already exists)

The operator needs to **manage** their server (allow/block players, capacity,
descriptor, announce, privacy) and **see** it (who's connected, how many, history,
their own credentials/setup). Most of the plumbing exists:

- **Write side = a signed control plane (built).** `AdminCommand` (kind 27422,
  operator-signed, audience-bound by a `["server", <runtime npub>]` tag and
  replay-deduped for the skew window — Spec 08 §9.0.1) is verified against the
  operator pubkey and applied live — over a relay (C2) or locally (`--admin`).
  The live `OperatorSnapshot` only reaches a channel-bound (QUIC) operator join. Today's verbs: `WhitelistAdd`,
  `WhitelistRemove`, `SetRequireSignin`.
- **Web surface (built).** Caddy already fronts the dedicated server's domain
  (`/ws` → game socket, else → the PWA). An authed `/admin` route is a small add.
- **Telemetry precedent (built).** `feedback_log.rs` is a bounded ring buffer —
  the pattern for a live roster; history needs an append-only on-disk log.

So Spec B is: **extend the command vocabulary**, add a **read side** (traffic +
credentials), a **thin auth**, and **two front-ends** over one shared core.

---

## 2. Architecture — one core, two faces

```
            ┌─────────────────────────── shared core ───────────────────────────┐
            │  ConsoleSnapshot (read-model, serde DTO)   AdminCommand (write)     │
            │  · credentials/setup    · live roster      · whitelist/blacklist    │
            │  · config               · traffic history  · cap / announce / name  │
            │  · current policy       · derived aggregates· kick / privacy         │
            └───────────────▲───────────────────────────────────▲────────────────┘
                            │ reads snapshot / emits commands     │
          ┌─────────────────┴───────────┐         ┌───────────────┴───────────────┐
          │  Web console (headless/VPS) │         │  In-game panel (self-host +    │
          │  SPA + JSON API on /admin   │         │  operator-as-player)           │
          │  auth: sign nonce w/ npub   │         │  auth: free (verified join)    │
          └─────────────────────────────┘         └────────────────────────────────┘
```

The **core** is the contract; both faces are thin presentation. Build the core
once; neither face contains policy logic.

---

## 3. The two faces

### 3.1 Web console — for headless / VPS servers (the flagship creator case)
A small admin SPA + JSON API the engine serves on a localhost admin port, fronted
by the **existing Caddy** at `/admin` (add a `reverse_proxy /admin* 127.0.0.1:<port>`
block to `Caddyfile.domain`; TLS already terminated). Renders the snapshot and
posts signed commands.

### 3.2 In-game panel — for self-host & operator-as-player
An "Operator" tab in the egui UI, **cross-platform** (native + WASM, since a web
player can be a dedicated server's operator). Visible only when the player's
verified npub equals the server's operator. Reads the snapshot in-process
(self-host) or via a snapshot-stream protocol message (operator joined a dedicated
server), and emits the same commands.

---

## 4. Auth — two paths, both reuse identity, no passwords

- **Web:** `/admin` issues a nonce → the operator signs it with the **operator npub
  via their bunker** (the same NIP-46 signer used everywhere) → server verifies the
  signature chains to the operator → session token (short-lived cookie). Identical
  trust check to the join proof, over HTTP. An impostor cannot sign for the
  operator npub, so cannot log in.
- **In-game:** **free.** The join proof already established the player's verified
  npub (`ServerPlayer.verified_pubkey`). If it equals the operator, the server
  unlocks the Operator tab and streams the snapshot to that player only.

---

## 5. Extended command vocabulary

Add to `AdminCommand` (kind 27422, operator-signed, applied via the existing path —
live within ~5s over relay, instant in-process):

| Command | Effect |
|---|---|
| `BlacklistAdd(pk)` / `BlacklistRemove(pk)` | **new** blocklist (`blocklist.txt`, npub per line, mirrors the allowlist) |
| `KickPlayer(pk)` | immediate disconnect of a connected player |
| `SetMaxPlayers(u16)` | live capacity change (feeds the §6 cap + the Card's `players` tag) |
| `SetAnnounce(bool)` | toggle Server-Card publishing — the Spec-A listing state transition |
| `SetServerName / SetAbout / SetRegion(String)` | descriptor edits → re-publish the Card |
| `SetPrivacy(level, retention)` | privacy policy — command + storage defined here; **values defined in Spec C** |

### Access precedence (allow + block resolved, deterministic)
1. npub in **blocklist** → reject (`Blocked`). Block always wins.
2. else **allowlist** non-empty and npub absent → reject (`NotAllowlisted`).
3. else `require_signin` and not signed-in → reject (`SignInRequired`).
4. else admit — subject to capacity (`AdmissionPolicy`, Spec A §6).

`KickPlayer` may optionally add a short-lived block to prevent instant rejoin
(a flag on the command; default off).

---

## 6. Traffic / telemetry — deliberately minimal

Tracking the least that is useful keeps Spec C small (less collected = less to
disclose and retain).

- **Live roster:** per connected player — verified npub, display handle, connect
  time, session length; plus current count and peak-today.
- **History:** an **append-only session log** (`npub`, `connect_ts`,
  `disconnect_ts`, `duration`) under the worlds/identity dir, plus derived
  **aggregates** (unique players today / 7d / all-time, peak concurrency, total
  sessions). Aggregates computed on read (or cached) from the log.
- **Explicitly NOT collected:** IP addresses, geolocation, behavioural/movement
  analytics. (Addable later behind a real need — but each addition expands Spec C.)

**GDPR boundary.** This is personal data (player presence over time). It is
**private to the operator** — served only over the authed console, **never**
published to public relays (a second reason telemetry is the authed API, not a
Card field). *What* is collected and *that it is operator-private* live here;
**retention, consent, and player-facing disclosure are Spec C.**

---

## 7. Credentials / setup view (read-only + actions)

Read: operator npub · runtime key pubkey · **delegation expiry + renewal
countdown** · announce state · configured relays · current connect-strings
(npub-only + host-pinned) + **QR** · server config (name / about / region / cap /
gamemode / ports / world) · identity-dir path · engine build + protocol version.

Actions: copy connect-string · show QR · **re-publish Card** · **renew delegation**
(bunker round-trip) · **announce on/off**.

All identity values render as **npub** (bech32), never hex (`npub-only display
rule`).

---

## 8. Module placement

```
game/engine/src/console/model.rs       # ConsoleSnapshot DTO (serde) — shared
game/engine/src/console/telemetry.rs   # session log + aggregates (native)
game/engine/src/console/http.rs        # admin JSON API + challenge auth (native)
game/engine/src/console/web/           # static admin SPA assets
game/engine/src/operator_panel.rs      # egui Operator tab (cross-platform)
game/engine/src/server_identity/admin.rs   # + new AdminCommand verbs + apply
game/engine/src/hosted_server.rs        # blocklist enforcement in join path (with §5 precedence)
game/engine/src/protocol.rs             # snapshot-stream message to a verified-operator player + reject reasons
```

Files stay small and single-purpose; the core carries the policy, the faces don't.

---

## 9. Testing + owner boundary

**Pure / unit (CI):**
- New `AdminCommand` verbs: parse / sign / verify / apply round-trips.
- Access precedence: block-wins, allowlist-gate, signin-gate, capacity — every
  branch.
- Session-log append + aggregate math (unique counts, peak concurrency, totals).
- `ConsoleSnapshot` serialization round-trip.
- Web-auth challenge: valid operator signature admits; wrong-key / replay rejects.
- Snapshot-stream gated to operator only (non-operator player gets nothing).

**Owner boundary (cannot verify solo):** live web-console login via a real bunker;
live in-game Operator panel on a 2-machine server; a real kick/block taking effect
across machines.

---

## 10. Out of scope (this spec)

- **Privacy semantics, retention windows, consent, player-facing disclosure** —
  Spec C (B only adds the `SetPrivacy` command + private storage).
- **Queue / one-in-one-out / Bitcoin-gated admission** — future, behind the Spec A
  §6 `AdmissionPolicy` seam.
- **Public global server directory** — deferred, its own spec.

---

## 11. Spec maintenance / follow-ups

- Bump `PROTOCOL` for the operator snapshot-stream message + any new reject reasons.
- Reflect the new `AdminCommand` verbs in the keystone design's admin table and in
  `tools/dedicated-server/README.md` (operator how-to: `/admin` URL, login flow,
  blocklist file).
- Note the `/admin` route addition in `Caddyfile.domain` and the admin port in the
  dedicated-server env table.

---

## 12. Implementation amendments — build 2026-06-17 (`worktree-server-creator-ux`)

Spec B was built in 8 TDD tasks, `check.sh` green throughout. The central refinement:

- **B-0 — the engine has NO HTTP server** (Cargo.toml: only `tokio-tungstenite`). So §3.1's
  "web console served by the engine at `/admin`" is **not buildable in-engine** without
  adding axum/hyper to the 20-TPS loop or a sidecar. **Decision (recommended, owner-reviewable):
  keep the engine HTTP-free.** The console is delivered through two engine-native channels:
  (1) **control** = the existing relay-signed `AdminCommand` path (kind 27422), extended with
  the new verbs (blacklist, kick, cap, announce, descriptor, privacy); (2) **telemetry (read)**
  = an in-game **Operator panel** fed over the game protocol to the verified-operator player —
  which also keeps player-presence data **off public relays** (GDPR-safe). A pure
  `console_auth::verify_console_login` primitive is built so a future console authenticates the
  operator with **no engine change**.

### Built + CI-tested (this branch)
- Extended `AdminCommand` vocab + `parse_cmd` + `apply_to` + `apply_admin_command_to_files`
  (blocklist.txt, console.json, kick queue) — tasks 1, 3, 4.
- Deterministic access precedence `access_policy::decide_access` (block > allowlist > sign-in),
  wired into `resolve_join_identity` — task 2.
- Operator-private telemetry `console_telemetry` (sessions + aggregates + retention purge,
  no IP/geo, never relayed) — task 6.
- `ConsoleSnapshot` read-model + `build_snapshot` — task 5.
- Operator snapshot-stream **wire**: `PacketType::OperatorSnapshot = 51` +
  `OperatorSnapshotPacket` + `is_operator` gate + **PROTOCOL 50→51** — task 7.
- `console_auth::{sign,verify}_console_login` — task 8.

### Owner boundary / deferred (logged)
- **B-7a — live operator snapshot stream:** the periodic server-side send (build a live
  snapshot + emit to operator-players every ~2s) + the client route/display. Untestable
  without a 2-machine operator-player; the wire + gate + panel **shell** are built.
- **Web operator panel:** `ConsoleSnapshot` + `operator_panel` are native; the web-operator
  path is a follow-up (like the Spec A web join).
- **Full web HTTP console + SPA** (B-0): an axum-in-engine vs `tools/sites/console/` sidecar
  **owner decision**; not built. The `console_auth` primitive is the seam it plugs into.
- **Live web-console login** (the HTTP challenge round-trip) — owner-boundary once a console
  surface exists.

### Merge note
This branch bumps `PROTOCOL` to **v51**; the `spec-48-electricity` worktree independently also
uses v51 — reconcile (renumber one) at merge.

### B-7a built — 2026-06-19
The deferred live operator snapshot stream is now wired (`a1cdb62`). `HostedServer::
broadcast_operator_snapshot` builds a `ConsoleSnapshot` from live state and sends
`OperatorSnapshotPacket` (~2s) to the verified-operator player only (`is_operator`
gate); the client stores the JSON and the native game loop renders `operator_panel`
in a collapsible egui window. Defensive (never panics the tick; no-op unless an
operator is in-game). Pure `roster_rows`/`build_snapshot`/`is_operator`/packet are
unit-tested; the live 2-machine display is the owner boundary. (PROTOCOL settled at
v52 at the 2026-06-18 merge, not v51 — the merge-note collision was reconciled.)
