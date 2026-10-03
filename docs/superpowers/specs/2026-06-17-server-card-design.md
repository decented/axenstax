# Server Card — npub→address resolution, descriptor & capacity advertising

**Date:** 2026-06-17
**Status:** DESIGN — approved in brainstorm, ready for an implementation plan. Spec
**A** of three (A = Server Card / discovery; B = Operator Console; C = Privacy &
tracking transparency). Build order A → B → C.
**Companion to:** [Heartwood-signed server identity (keystone)](2026-06-17-heartwood-signed-server-identity-design.md)
· [Server operator/creator identity model](2026-06-17-server-operator-identity-creator-model.md)
· [Dedicated Docker server](2026-06-16-dedicated-docker-server-design.md)
**Constraints honoured:** `feedback_signet_boundary` (no AxeNStax-specific Signet
changes — this is plain Nostr events), `project_identity_default_nonpublic`
(announcing a server is explicit opt-in, never automatic), `project_shared_infra_strategy`
(the resolver/Card format is a generic primitive), `reference_trotters_relay`
(`wss://relay.trotters.cc` is our own infra), and the non-custodial settlement
canon (ADR-004 / Spec 6 §1.5) for the deferred Bitcoin-admission door.

---

## 1. The question this answers

A creator runs a server and shares **one stable handle** — their operator npub.
Players reach the server by that npub even though its IP can change, see its name
/ blurb / how full it is *before* joining, and the whole thing remains
cryptographically tamper-proof (a hostile relay cannot redirect a player to an
impostor). Today the connect-string bakes in a fixed `host:port`; this spec adds
the indirection layer — **npub → current address** — and the public descriptor
that rides with it.

Think of it as **DNS, but the record is signed by the operator and published to
Nostr relays instead of a nameserver.**

---

## 2. Server states — announcing is opt-in

A server is in exactly one of three states. Identity and announcement are both
**additive trust upgrades**, never a gate to running a server.

| State | Has identity? | Publishes a Card? | How players reach it |
|---|---|---|---|
| **Anonymous** | no | no | raw address (`ws://host:port`) — today, unchanged |
| **Verified, unlisted** | yes | no | operator shares the connect-string directly (point-to-point); zero relay footprint |
| **Verified, announced** | yes | **yes** | resolvable by **npub alone**; appears in the player's "My Servers" list |

Pairing produces a **Verified, unlisted** server. Moving to **announced** is an
explicit operator action (`--announce` / a Console toggle) — going public is
never automatic, per `project_identity_default_nonpublic`.

---

## 3. Two events, two cadences

Resolution rides on two signed Nostr events whose update frequencies differ — and
that difference is *why* there are two.

### 3.1 Anchor — the existing attestation (kind `30420`, operator-signed)

Already defined and built by the keystone design. **Operator-signed**,
addressable, published under the **operator pubkey**, found by
`{kinds:[30420], authors:[<operator hex>]}`. It already names the delegate runtime
key (`["d","<runtime pubkey hex>"]`) and carries an allowed-signing-kinds list.

**This spec's changes to the Anchor:** (a) add `["k","30422"]` to its allowed
kinds (authorising the runtime key to publish Address Cards); (b) add one or more
`["relay", "<url>"]` hint tags naming where the Card is published (so a client that
found the Anchor on the default discovery relay knows where to look for the Card);
and (c) **publish it to relays** at pairing and at each renewal. The operator key
signs the Anchor — but only at those rare bunker-online moments.

### 3.2 Address Card — new (kind `30422`, runtime-signed)

The frequently-updated record the **server box signs itself, with the runtime key,
no bunker needed**. Addressable (latest-wins).

```jsonc
{
  "kind": 30422,
  "pubkey": "<runtime pubkey hex>",
  "created_at": <unix>,
  "tags": [
    ["d", "server"],                              // replaceable key (one Card per runtime key)
    ["op", "<operator pubkey hex>"],              // back-reference; cross-checked, not trusted
    ["endpoint", "wss://play.example.com:8080/ws"], // ordered; multiple allowed (failover)
    ["endpoint", "wss://203.0.113.7:8080/ws"],
    ["name", "Cool Creeper SMP"],
    ["about", "Family-friendly survival, no griefing"],
    ["region", "eu-west"],
    ["players", "12", "20"],                      // current (best-effort) , max
    ["protocol", "51"],                           // engine protocol version
    ["privacy", "unset"]                          // RESERVED — Spec C defines values + retention
  ],
  "content": ""                                    // all data in tags; content reserved
}
```

**Why the box signs the Card and not the operator:** a home server (a kid hosting
from their bedroom) can get a new public IP daily. Routine address updates **must
not** require the bunker, or runtime-independence (keystone Decision #2) breaks.
The operator key only ever signs the rare Anchor; the disposable runtime key signs
everything that changes often.

---

## 4. Resolution flow + the anti-redirect guarantee

Client holds an operator npub:

1. Fetch the **Anchor**: `{kinds:[30420], authors:[operator hex]}` → newest valid
   (signature by operator, not expired). Read the delegate runtime pubkey; confirm
   it authorises kind `30422`.
2. Fetch the **Card**: `{kinds:[30422], authors:[delegate runtime pubkey]}` →
   newest; cross-check its `["op", …]` equals the operator. Read endpoints +
   descriptor + capacity + protocol.
3. Dial the endpoints in order; on the first that connects, run the **existing
   join proof** (Track 3), which independently re-verifies
   `runtime key → attestation → operator npub`.

**The guarantee:** relays are an *untrusted hint layer*. A hostile relay can drop
or delay events, or hand back a forged Card pointing at an impostor — but the
impostor cannot produce a valid join proof chaining to the expected operator npub,
so the client simply **fails to join** rather than being silently redirected.
Trust lives in the join proof (already built), never in the relay.

**Caching.** The client stores the last successfully-resolved `{endpoints,
descriptor}` per operator npub. If relays are unreachable, it falls back to the
cached endpoint (still proof-verified on connect), so a relay outage never strands
a player from a server they already know.

**Relays — non-circular discovery order.** The client always knows one default
**discovery relay**, `wss://relay.trotters.cc` (our infra), and looks there for the
**Anchor**. The Anchor's `["relay", …]` hints then tell it where the **Card** lives
— so an operator using custom relays is still resolvable without the client
pre-knowing those relays. The box publishes the Card (and the Anchor) to the
default relay plus any `--card-relays` / `AXENSTAX_CARD_RELAYS` (comma list), and
sets the Anchor's relay hints to that same publish set. If the hints yield nothing,
the client falls back to the default relay, then to its local cache.

---

## 5. Connect-string forms (all coexist)

`connect.rs` gains npub-only parsing; existing forms are untouched.

| Form | Behaviour |
|---|---|
| `axenstax://npub1…` | **new** — resolve via relays (§4) |
| `axenstax://host:port#op=npub` | existing — host pinned, skip relay lookup, still proof-verified |
| `ws://host:port` / `wss://host/ws` | existing — anonymous, no proof |

`ConnectInfo` gains a `resolve_by: Option<OperatorNpub>` variant so a caller knows
whether to dial directly or run resolution first.

---

## 6. Capacity + admission seam

**Cap (already built — surfaced + future-proofed here).** `--max-players` /
`AXENSTAX_MAX_PLAYERS` (default 8) already configures the cap, and `ws_transport`
already rejects connections once it's reached. This spec adds:

- **Advertising:** the Card's `["players","<cur>","<max>"]` tag, refreshed on the
  heartbeat republish, so "My Servers" rows show fullness *before* a click.
- **A clean rejection reason** delivered to the client (a `ServerFull` reject code)
  so the UI says "Server full", not a silent drop.

**Admission seam (designed now, one variant built).** Lift the inline
`current >= max → reject` check into a single `AdmissionPolicy` decision point:

```
enum AdmissionPolicy { HardCap }   // only variant today
fn decide(policy, ctx) -> Admission  // Admit | Reject(reason)
```

- `HardCap` — **implemented now** (exactly today's behaviour, relocated to the seam).
- `Queue` / `OneInOneOut` — **future** (Spec B / beyond): hold arrivals when full,
  admit as slots free, optional priority for whitelisted players.
- `BitcoinGated` — **future, large build**: pay-to-enter / priority-for-payers.
  Not built here; the seam must not preclude it. Lands on existing canon — the
  non-custodial settlement decision already blesses a **non-refundable one-way
  entry as operator revenue** (ADR-004 / Spec 6 §1.5), so this is that door wired
  to this seam, not a new policy debate.

The seam is concrete (a typed decision point), not a buried conditional — per the
"concrete, not cards" rule.

---

## 7. "My Servers" — the local player list

The client persists servers the player has joined; **no public directory** (that
was the deliberate scope choice — directed resolution + local memory, not a global
index). Each entry:

```
{ operator_npub, name, last_endpoint, last_joined_unix, favourite: bool }
```

Stored in the existing client save location (`my_servers.json` native; the web
save path on WASM — no new infra). The entrance/lobby gains a **"My Servers"**
view: rows show server name + verified badge + last-seen + `cur/max`; selecting a
row re-resolves fresh (§4) and joins. Adding a server happens implicitly on a
successful join, or explicitly by pasting an `axenstax://npub1…` string.

---

## 8. Platform split (load-bearing)

Publishing is native-only (servers are native); **resolution and "My Servers" must
run on WASM** (the web client joins servers too). So:

| Piece | Where | Platform |
|---|---|---|
| Card **format + build + verify** (pure) | `server_identity/server_card.rs` | cross-platform shared |
| **Publish** (boot + heartbeat + address-change) | `server_main` publisher (nostr-sdk) | **native only** |
| **Resolve** (selection logic) | `server_resolve.rs` (pure pick-newest/cross-check) | cross-platform shared |
| Relay I/O for resolve | `RelayQuery` trait: native = nostr-sdk; web = JS relay bridge reusing the Stash/Signet relay machinery | per-platform impls |
| **"My Servers"** store | `my_servers` | cross-platform (platform persistence) |

`server_identity/` stays `#![cfg(not(target_arch = "wasm32"))]`; the new
cross-platform pieces (`server_card` verify, `server_resolve`, `my_servers`) live
outside that gate or are split so the pure parts compile on WASM.

---

## 9. Publishing cadence (server side)

The endpoint advertised is derived from existing config — `--public-host` /
`AXENSTAX_PUBLIC_HOST` + `--port` (scheme `wss` when fronted by Caddy/TLS, `ws`
when bare). The publisher fires:

- **On pairing / renewal:** (re)publish the Anchor (operator-signed, via the
  Console's bunker round-trip) with `["k","30422"]` in allowed kinds.
- **On boot** (if announced) and **on a heartbeat** (default every few minutes,
  configurable) and **on detected address change:** republish the Card
  (runtime-signed, no bunker), refreshing `players` and `created_at`.

A server that is paired but **not** `--announce`d publishes nothing.

---

## 10. Module placement

```
game/engine/src/server_identity/server_card.rs   # format + build + verify (shared pure)
game/engine/src/server_main.rs                    # publisher hook (native): boot/heartbeat/change
game/engine/src/connect.rs (server_identity)      # + npub-only parse → ConnectInfo::resolve_by
game/engine/src/server_resolve.rs                 # resolver: RelayQuery trait + pure selection (shared)
game/engine/src/my_servers.rs                     # local list store (shared, platform persistence)
game/engine/src/ws_transport.rs                   # AdmissionPolicy seam (HardCap) + ServerFull reject
game/engine/src/protocol.rs                        # ServerFull reject reason (if not already representable)
```

Files stay small and single-purpose; no file is expected past ~300 lines.

---

## 11. Security properties

- **Relay = untrusted transport.** Substitution/redirect is defeated by the join
  proof (§4), not by trusting the relay.
- **Anchor unforgeable** — operator-signed; a forged Anchor fails signature check.
- **Card authority bounded** — a runtime key may only publish a Card the Anchor
  authorised (`["k","30422"]`); the resolver checks the Anchor first.
- **Freshness** — addressable/replaceable semantics keep only the latest event per
  `(kind, author, d)`; clients take newest by `created_at`. A stale Card just
  points at an old endpoint → connect fails → fall back to cache/other relays.
- **No private data in the Card** — it is public by definition; the `privacy` tag
  is metadata about the *server's policy*, not about any player.

---

## 12. Testing

**Pure / unit (CI):**
- Card serialize → sign (runtime key) → verify round-trips; tampered tag fails.
- Anchor authorises `30422`; resolver rejects a Card whose `op` ≠ expected operator.
- Resolver picks newest by `created_at`; handles missing Anchor, missing Card,
  stale Card, multiple endpoints (order preserved).
- `connect.rs` parses `axenstax://npub1…` → `resolve_by`; existing forms unchanged.
- `my_servers` add (implicit on join) / explicit-add / remove / favourite / dedupe
  by npub.
- `AdmissionPolicy::HardCap` admits below cap, rejects at cap with `ServerFull`.
- Impostor endpoint (valid Card shape, wrong operator chain) → join proof rejects
  (extends Track 3 coverage).

**Owner boundary (cannot verify solo):** a live relay publish from one machine and
a real cross-machine **resolve-by-npub** join from another (browser + phone for the
web path), confirming the chain end-to-end. Build + green all unit tests, then stop
and hand off.

---

## 13. Explicitly out of scope (this spec)

- **Public global server directory / browser** — deferred; its own spec once basics
  are proven (curation, ranking, anti-spam).
- **Queue / one-in-one-out / Bitcoin-gated admission** — future variants behind the
  §6 seam; not built here.
- **Privacy semantics, retention, player-facing disclosure, consent** — Spec C
  (this spec only reserves the `privacy` tag so the wire format is stable).
- **Operator Console UI, traffic/usage analytics, credential view** — Spec B.

---

## 14. Spec maintenance / follow-ups

- Register kind **`30422`** (and confirm `30420`) in `forgesworn/nips` once built
  (matches the keystone design's existing `30420` registration follow-up).
- When built, reflect the new wire kind + tag schema in
  `docs/spec/04-networking.md` (protocol/version table) and note the connect-string
  forms in the operator docs (`tools/dedicated-server/README.md`).
- ~~Bump `PROTOCOL`~~ — **not needed** (amendment A-1): the Card is off-band Nostr, the
  npub connect-string is client-side, and "Server full" reuses the existing
  `JoinRejectPacket.reason`. No wire-format change in Spec A.

---

## 15. Implementation amendments — build 2026-06-17 (`worktree-server-creator-ux`)

Spec A was built in 11 TDD tasks, `check.sh` green throughout. The design held; these
are the refinements (full detail + per-task log in the build charter
`docs/goals/2026-06-17-server-creator-ux-buildout.md`):

- **A-1 — No `PROTOCOL` bump** (above): no wire-format change.
- **A-2 — Card verify doesn't enforce the attestation's `allowed_kinds`** (parity with
  the shipped `claim.rs`); pairing/attestation format untouched. Trust still holds — the
  signer must be the attested runtime key, and the operator is cross-checked.
- **A-3 — Attestation relay-hint tags deferred.** Publisher + resolver use the configured
  relay set (default `wss://relay.trotters.cc` + `--card-relays`).
- **A-4 / A-6 / A-7 — `nostr` is native-only** (Cargo.toml). So the resolver core works on
  a lightweight serde_json `RawEvent` (cross-platform) and does **parse + select, no
  signature verification** — per §4 the join proof is the trust anchor (a forged relay
  record causes at worst a *failed* join). The web path reuses the page's existing
  `axenstax_npub_to_hex` (npub-decode.js) + a new `axenstax_relay_query` WebSocket bridge
  (`relay-query.js`, served from the site's `/static/`).
- **A-5 — `server_card.rs` stays native-gated** (like `claim.rs`); a cross-platform
  `parse_card` isn't needed because the resolver core is independent of it.
- **A-8 — Native join-by-npub is wired; three owner-boundary follow-ups deferred:** the
  **web** join handler (the sync wasm handler needs an async restructure — Direct join
  works on web today), the native resolve **UI-block** (≤5s; an async "Connecting…" screen
  is the fix), and **record_join on Direct joins** (no card → no server name).
- **A-9 — "Server full" textual reject reason deferred.** The cap is enforced through the
  `admission::decide` seam *before* the WS upgrade (a silent drop — no channel for a reason
  there). Enforcement is unchanged; surfacing the reason needs an accept-loop restructure.
- **A-10 — My Servers panel** is a right `SidePanel` on a ≥1360px breakpoint; the
  add-by-npub field was omitted (servers add implicitly on a successful join). Placement /
  width / feel are playtest-tunable.
- **Follow-up — live player count.** The publisher advertises `0/max`; wiring the main
  loop's live count into the heartbeat republish is a follow-up.

**Owner-boundary live tests (unchanged from §12):** real bunker pairing, a live relay
publish, and a 2-machine resolve-by-npub join (browser + phone for the web path, whose
join wiring is the A-8 follow-up).
