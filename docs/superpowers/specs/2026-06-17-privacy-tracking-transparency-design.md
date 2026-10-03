# Privacy & tracking transparency — declaration, disclosure, retention

**Date:** 2026-06-17
**Status:** DESIGN — approved in brainstorm, ready for an implementation plan. Spec
**C** of three (A = Server Card / discovery; B = Operator Console; C = Privacy &
tracking transparency). Build order A → B → C.
**Companion to:** [Server Card (Spec A)](2026-06-17-server-card-design.md) ·
[Operator Console (Spec B)](2026-06-17-operator-console-design.md) ·
[Heartwood-signed server identity (keystone)](2026-06-17-heartwood-signed-server-identity-design.md)
**Constraints honoured:** `project_identity_default_nonpublic` (default is **no
tracking**), the non-custodial ethos (the platform holds no player data; the
**operator** is the data controller), kid-safe audience (plain-language notices),
`npub-only display rule`.

---

## 1. What this spec is

Spec A reserved a `privacy` tag on the Server Card; Spec B added the `SetPrivacy`
command + operator-private telemetry. This spec fills in the **meaning**: the
privacy levels, what each controls, how a player is told *before and at* join, how
long data is kept, who is responsible, and — the part flagged in the brainstorm —
**how honest the "we don't track you" claim really is.**

---

## 2. Default: no tracking

A brand-new server is `none` — **nothing about players is persisted.** The live
roster exists only in memory while a player is connected (transient processing
needed to run the game and kick a current griefer) and is gone on disconnect or
restart. The operator opts **into** history explicitly. The common case therefore
needs no consent flow at all.

---

## 3. Privacy levels (the Card tag + `SetPrivacy` values)

The declaration is two fields: a **level** and a **retention window**. Encoded in
the Card's `privacy` tag and carried by the `SetPrivacy` command.

| Level | Card tag | What is persisted | Player notice |
|---|---|---|---|
| **`none`** (default) | `["privacy","none"]` | nothing — live roster is in-memory only | none needed |
| **`sessions`** | `["privacy","sessions","<days>"]` | the §6/Spec-B session log (npub, connect/disconnect, duration), auto-purged after `<days>`; only anonymous aggregate counts persist longer | pre-join badge + one-time join notice |

That is the whole taxonomy — two real levels plus a retention number. It maps
directly to what Spec B collects; there is nothing else to declare.

**Never collected at any level** (from Spec B's scope choice): IP addresses,
geolocation, behavioural/movement analytics.

---

## 4. What each level controls (wired to Spec B telemetry)

- **`none`:** Spec B's append-only session log is **disabled** — no disk writes;
  aggregates are not persisted. A restart is a clean slate.
- **`sessions:<N>`:** the session log is **enabled**; a purge pass (on a timer and
  at boot) drops entries older than `N` days; anonymous aggregates (unique counts,
  peak concurrency) persist as plain numbers with no npub attached.

Switching `sessions → none` via the console **purges the existing log immediately**
(you can't declare "no tracking" while sitting on retained records).

---

## 5. Player-facing disclosure

- **At server-selection (pre-join):** the badge derived from the Card's `privacy`
  tag, shown in the "My Servers" list and the connect flow —
  **"🔒 No tracking"** or **"📋 Keeps session history · 30 days"** — so the player
  knows *before* committing. (This is the "when people choose a server, they can
  tell whether it tracks them" requirement.)
- **At join (tracking servers only):** a one-time, plain-language notice —
  *"This server keeps a record of when you play, for 30 days, so the owner can
  moderate. Join anyway?"* — decline = don't join. `none` servers show nothing.
  The acknowledgement is remembered per-server (stored on the "My Servers" entry)
  so it doesn't nag every join; it re-prompts if the server's posture changes.
- **Wording is kid-safe** — no legalese; the audience is children.

---

## 6. Retention & erasure (data-subject rights)

- **Retention:** `sessions:<N>` auto-purges entries older than `N` days. `N` is
  operator-set (sensible default e.g. 30); the value is disclosed in the badge.
- **Erasure:** two privacy commands extend Spec B's vocabulary (operator-signed,
  kind 27422):
  - `ForgetPlayer(npub)` — purge all of one player's session records (right to
    erasure on request).
  - `PurgeAllHistory` — wipe the whole session log.
- A `none` server has nothing to export or erase.

Realistically a player cannot query a third-party operator's box directly, so the
model is **pre-join transparency + operator tooling**: the player decides up front
from the badge/notice, and the operator has the tools to honour an erasure request.
Stated honestly rather than promised as automatic.

---

## 7. Who is responsible (data-controller allocation)

- The **operator** runs the box and holds any telemetry, so the **operator is the
  data controller** and carries the GDPR obligations. The engine gives them safe
  defaults and the tools (retention, purge, erasure, disclosure) to comply.
- **AxeNStax / the platform is not a controller** of player-presence data — it
  never receives it (telemetry is operator-private, never relayed; Spec B §6). This
  mirrors the non-custodial posture: the platform provides primitives and holds
  nothing, exactly as with funds.

This is product/architecture guidance, not legal advice or a certification.

---

## 8. The honest verification model (the brainstorm's open question)

**Cryptography proves the operator *declared* a posture — not that the box
*behaves* that way.** You cannot cryptographically prove a remote server isn't
logging. So assurance comes in three layers of increasing strength:

- **L1 — Signed declaration (this spec ships it).** The privacy level rides in the
  runtime-signed Card, chaining to the operator npub. The posture is therefore
  **non-repudiable and attributable** — the operator has publicly committed under
  their own reputation. Lying becomes an attributable act (reputational + legal
  exposure), not a silent one.
- **L2 — Open-source default build (already true).** The canonical server binary is
  open-source; `none` in the default build genuinely persists nothing, auditable by
  anyone. A self-hoster *can* fork and change it — but then they have diverged from
  the audited binary.
- **L3 — Attested build (future, reserved, NOT built here).** A reproducible-build
  hash embedded in the attestation would prove the box runs the *unmodified audited
  binary*, upgrading "declared honest" to "provably running the honest build." This
  is the real technical answer to "genuinely verify"; C reserves the path and does
  not build it.

**Bottom line for the UI:** the **identity** is *verified* (cryptographic), but the
**privacy posture** is *declared* (a signed promise). The badge wording must keep
that distinction — e.g. a verified-identity tick next to "Owner says: no tracking",
never implying the privacy claim is independently proven. Honesty about the limit
is the feature.

---

## 9. Module placement

```
game/engine/src/privacy.rs                 # PrivacyLevel type, Card-tag <-> level encoding,
                                            #   pure retention-purge selection, session-log gating
game/engine/src/console/telemetry.rs       # honour level: gate writes (none), run purge passes
game/engine/src/server_identity/admin.rs   # + ForgetPlayer / PurgeAllHistory commands
game/engine/src/my_servers.rs              # store per-server privacy acknowledgement
client UI (cross-platform)                  # privacy badge in My Servers/connect; pre-join notice modal
```

The privacy *logic* (encoding, purge selection, gating) is pure and cross-platform;
the disk telemetry it gates is native (servers are native); the badge/notice are
client UI.

---

## 10. Testing + owner boundary

**Pure / unit (CI):**
- `PrivacyLevel` ↔ Card `privacy` tag encoding round-trips (`none`,
  `sessions:<N>`); malformed tag → safe default (`none`).
- `none` gates the session log: zero disk writes occur.
- Retention purge selects exactly the entries older than `N` days; boundary days
  handled.
- `sessions → none` triggers a full purge.
- `ForgetPlayer(npub)` removes exactly that npub's records and nothing else;
  `PurgeAllHistory` empties the log.
- Badge string derives correctly from a Card; pre-join notice fires only for
  tracking levels and respects a remembered acknowledgement; re-prompts on posture
  change.

**Owner boundary (cannot verify solo):** a player on machine B sees the correct
badge + notice for machine A's declared posture; an erasure request verified to
clear the right records across a real session log.

---

## 11. Out of scope (this spec)

- **Attested / reproducible-build remote attestation (L3)** — future; this spec only
  reserves the path.
- **Any platform-side data handling** — the platform holds nothing.
- **Legal advice / certification** — this is product guidance; operators remain
  responsible as data controllers.
- **Richer telemetry** (IP/geo/behaviour) — not collected; adding any would expand
  this spec.

---

## 12. Spec maintenance / follow-ups

- Reflect the `privacy` tag encoding in the Spec A Card schema (keep A and C in
  lockstep on the tag format).
- Document the operator privacy controls + the L1/L2/L3 assurance model in
  `tools/dedicated-server/README.md`, and the player-facing badge meaning in the
  player help / `.com/safety` parent page.
- If kind `30422` (the Card) gains/loses privacy fields, bump the Card version note
  in A and re-check this spec.

---

## 13. Implementation amendments — build 2026-06-17 (`worktree-server-creator-ux`)

Spec C was built in 5 TDD tasks, `check.sh` green throughout. **Default = no tracking**
held. The model maps cleanly onto the pieces built in Specs A + B:

### Built + CI-tested (this branch)
- `privacy.rs` — `PrivacyLevel{None, Sessions{retention_days}}` + `encode`/`decode` (the Card
  `privacy` tag `none` / `sessions:<days>`), `from_settings` (the `ConsoleSettings` bridge),
  `badge`, `should_persist`, `retention_cutoff`, `should_show_notice` (task C1/C2/C4).
- `console_telemetry` gated by level — `record_connect_gated` (None ⇒ nothing persisted) +
  `apply_retention` (purge by cutoff) + `forget_player`/`clear` erasure (C2/C3).
- `AdminCommand::ForgetPlayer`/`PurgeAllHistory` + the file-apply erasure path (C3).
- `my_servers` per-server `privacy_ack` + `should_show_notice` decision (C4).
- Publisher wiring — the `--announce` Card now carries the **configured** privacy (via
  `privacy::encode(from_settings(..))`) + descriptor/cap from `ConsoleSettings`, defaulting to
  `none` when unset; the announce gate honours a persisted `SetAnnounce` (C5).

### Owner boundary / deferred (logged)
- **Live telemetry capture:** wiring `record_connect_gated` on a real join + `apply_retention`
  on the reload — needs a live server (the capture itself is owner-boundary).
- **Player-facing notice MODAL + badge display** in the My Servers / connect UI — visual; the
  *decision* (`should_show_notice`) + the badge *string* + the ack store are built.
- **L3 attested reproducible build** — the future "provably running the audited binary" tier;
  this spec reserves it (§8). L1 (signed declaration) ships via the Card privacy tag; L2 is the
  open-source default build.

### The honest line (§8) — unchanged
**Identity is VERIFIED (crypto); the privacy posture is DECLARED (a signed promise).** The UI
must never imply the privacy claim is independently proven. That distinction is the feature.

### C live capture built — 2026-06-19
Live telemetry capture is now wired (`877ecce`). `HostedServer` owns the `SessionLog`;
it records a verified remote player's connect on join + disconnect on leave through
pure `console_telemetry::capture_{connect,disconnect}` (gated by identity + privacy
level), and `server_main` refreshes the level + applies retention + persists in the
~5s reload. Default `PrivacyLevel::None` (no-tracking) records nothing — default-safe.
The gate logic is unit-tested; live capture against a real remote join is the owner
boundary.
