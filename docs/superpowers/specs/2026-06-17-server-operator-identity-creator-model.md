# Server Operator Identity — Creator Verification & Key Choice

**Date:** 2026-06-17
**Status:** MODEL + POLICY = CANON. Underlying mechanism BUILT (Tracks 2–6 + C1 +
C2, see the keystone design). Creator-facing verification UX = partially built
(proof is enforced; the in-product badge + share flow remain). No new mechanism
is proposed here — this doc fixes the *operator/creator identity model* so it
survives a rebuild, and scopes the small UX tail.
**Companion to:** [Heartwood-signed server identity (keystone design)](2026-06-17-heartwood-signed-server-identity-design.md)
and [Dedicated Docker server](2026-06-16-dedicated-docker-server-design.md).
**Constraints honoured:** `feedback_signet_boundary` (Signet changes stay
general-purpose), `project_identity_default_nonpublic` (identities default
non-public; going public is explicit opt-in), `project_shared_infra_strategy`
(don't hard-code AxeNStax assumptions into the identity primitive).

---

## 1. The question this answers

*Who runs a server, and how does a player know it's really them?* A creator
(say a YouTuber) wants viewers to join **their** server and be sure it isn't an
impersonator farming their audience. This doc is the definitive answer to how a
server gets — or deliberately doesn't get — a verifiable operator identity, what
device/app signs it, and which key a creator should use.

It is **operator/creator policy + model**, layered on the cryptographic
mechanism the keystone design already built. Nothing here changes the wire
format or the trust algorithm.

---

## 2. The identity model (canon)

A server is in exactly one of two states. **Identity is purely additive** — it is
a trust *upgrade*, never a gate to running a server (keystone Decision #5).

### 2.1 Anonymous server — *no keys at all*
Never paired ⇒ no `.identity` dir ⇒ `ServerIdentity::load` returns `None` ⇒ the
server boots with `identity = None` (`server_main.rs`,
`BootIdentity::Anonymous`). It runs the full world sim and accepts joins; it just
sends **no** join proof (`identity_proof()` returns `None`, so
`JoinAccept.server_identity` is empty). Players connect by address and trust it
exactly like joining any game server by IP today. **An anonymous server has
neither an operator npub nor a runtime key — it is genuinely identity-less.**

### 2.2 Verified server — *two keys*
Pairing brings the server's own key into being and ties it to a public identity:

| Key | Where it lives | Public? | Role |
|---|---|---|---|
| **Operator npub** | the operator's bunker (Heartwood / phone / …) | **published** (the anchor players check) | signs the delegation attestation, admin commands, claims |
| **Runtime key** | on the server box (`<worlds>/.identity`, mode 0600) | never published | the disposable, delegated day-to-day signer; chains *up* to the operator via the attestation |

A joining client verifies the chain: *the runtime key signed my fresh nonce* **and**
*the attestation authorises that runtime key* **and** *the attestation is by the
operator npub I expected*.

### 2.3 Two properties that matter for creators
- **Bunker-agnostic** (Decision #1). Any NIP-46 signer can be the operator key —
  the vendored `signet-nip46-client` is generic.
- **Runtime independence** (Decision #2). The bunker is only touched at **pairing**
  and **renewal**; thereafter the box signs locally with the runtime key. The
  operator's phone/Heartwood can be offline, in a pocket, or dead and the server
  keeps running *and* keeps proving its identity until the delegation expires
  (default 90 days).

---

## 3. Who holds the operator key — signer options

All of these pair via the same `nostrconnect://` QR; the choice is **where the
secret lives**, a security-vs-convenience trade. They are functionally identical
for producing the signature.

| Signer | Convenience | Security | When |
|---|---|---|---|
| **Heartwood** (Pi / ESP32 HSM) | scan a QR; dedicated device | highest — key never leaves a single-purpose, air-gappable device | recommended for a public, monetised server |
| **Phone bunker** — Amber, nsec.app | already in your pocket | key on a general-purpose device (more attack surface) | convenient; fine for most |
| **Signet** (the platform's own phone identity, `mysignet.app`) | the *same* identity players sign in with | per Signet's model | natural fit — one identity, both player and operator roles |
| **Self-hosted signer** | your infra | you own it | advanced operators |

**A phone is a fully supported first-class option** — a creator can pair with
Amber / nsec.app / Signet today and it works exactly like a Heartwood. The
Heartwood is the "keys never leave a dedicated box" upgrade for when real money
or a large audience is at stake.

---

## 4. The creator verification flow

1. The creator holds a Nostr identity (npub) in their chosen bunker.
2. They **publish that npub** wherever fans look — YouTube *about* page, website,
   video description: *"My official server runs under `npub1abc…` — verify it."*
3. One-time `--pair-server` on the box → it prints a `nostrconnect://` QR → the
   creator approves in their bunker → the bunker signs the delegation attestation.
4. They share the connect-string (`--show-connect` →
   `axenstax://host:port#op=npub1abc…`) with viewers, or viewers paste the npub.
5. On join, the viewer's client (C1) verifies the server's proof chains to that
   npub and shows **"✓ verified — [creator]'s server"**, or **refuses** if it
   doesn't.

**Anti-impersonation:** nobody can stand up a fake *"[Creator]'s Official
Server"* and harvest their audience, because they can't produce a proof chaining
to the real npub — they don't hold the key. **One published npub verifies every
server that creator runs** — their operator identity is portable.

---

## 5. Key choice — shared gamer key vs dedicated operator key

> **There is no technical constraint forcing a separate key.** The attestation
> chains to whatever npub the operator pairs with. A creator *may* use their
> everyday "gamer" npub. If they then join their own server, they appear as both
> the **operator** *and* a **verified player** under that one npub — the engine
> checks those in different places, so there is no conflict.

It is a **key-hygiene decision**, because the two roles stress a key differently:

| | Gamer key | Operator key |
|---|---|---|
| Usage | hot — signs every join, owns in-game economy blocks, on the device they play with | cold — signs once at pairing, then occasional admin/claims |
| Stakes | their play identity & history | server authenticity + (eventually) money flow |

Reusing one key = **one compromise point for both roles**: a leak of the hot
gaming key could then impersonate the official server and (once payouts exist)
sign money claims. Separate keys give blast-radius isolation, clean **rotation**
(retire the operator key without disturbing gaming history), and a sensible
**brand-vs-personal** split. (Note: even with a shared key it never lands on the
box — the runtime-key delegation means it signs once at pairing. The risk is
"one key, two hats", not "key on a VPS".)

This fits the platform's existing **multi-persona model** (parent accounts
controlling child accounts, persona separation, non-public-by-default) — a
creator legitimately having both a personal gamer npub *and* a published
official-operator npub is the model working as intended, not an awkward extra.

### Recommendation
| Operator | Recommendation |
|---|---|
| Small creator, just wants recognition, no money | **reuse the gamer npub** — simplest, recognition is automatic |
| Public figure / audience / money flowing | **dedicated operator (brand) npub**, published as "official server identity" — security + separation, at the cost of one published statement |

The system is indifferent — this is purely *which npub the operator points the
bunker at*. Operator-facing docs should present this trade and default the
guidance to a dedicated key for monetised servers.

---

## 6. Status: built / remaining / boundary

**Built** (see keystone design + the C-series follow-ups, `main`):
- Pairing, delegation attestation, runtime key, anonymous/verified boot (Tracks 2–6).
- **C1** — client enforces the join proof against a pinned `#op=` operator
  (refuse on mismatch; `verified_operator` captured). Native-only.
- **C2** — operator admin commands over a relay (`--admin-sign` / `--admin-publish`
  → server applies live).

**Remaining buildable — the creator-facing verification UX** (the value tail this
doc scopes):
1. **Verified-server badge.** C1 stores `verified_operator` but only logs it.
   Surface it in the Join dialog and (eventually) a server list:
   "✓ verified — `npub…`" or a clear refusal reason. ~small.
2. **Share-my-server flow.** Reuse the existing QR renderer (`pairing::print_qr` /
   the lobby QR) to show the operator their `axenstax://…#op=` connect-string +
   QR for pasting into a video/description. ~small.
3. **Operator-docs guidance** on the §5 key choice (dedicated key for monetised
   servers) and the §3 signer options.

**Owner boundary (cannot be verified solo):** a live pair (Heartwood *or* phone)
+ a 2-machine pinned join that verifies and refuses a wrong operator.

**Upstream:** register kinds `30420` (attestation) / `27421` (claim) / `27422`
(admin) in `forgesworn/nips` as general-purpose Signet primitives.

---

## 7. Open decisions (owner)

- **Build the verified-server badge now, or wait for a consumer?** The badge is
  the first real consumer of C1's verification result; everything else (worlds
  leaderboard, audit) is later. Likely worth doing alongside the first real
  monetised-server use.
- **Default operator-key guidance** baked into docs — dedicated key for monetised
  servers (recommended) vs leave it open.
- **How a dedicated operator npub is publicly tied to a creator** — for now a
  manual published statement (YouTube/site). A verifiable link mechanism
  (e.g. NIP-05 / a signed claim) is a later, general-purpose Signet question, not
  AxeNStax-specific.

---

## 8. Spec maintenance

Per `CLAUDE.md`: when the verification-UX items in §6 ship, update this doc and
the keystone design's status table; reflect any new operator commands/flags in
`tools/dedicated-server/README.md`. The §2 model and §5 key-choice policy are
canon — keep them current if the mechanism changes.
