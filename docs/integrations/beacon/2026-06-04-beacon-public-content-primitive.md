# Beacon — a public-content-distribution primitive for Nostr (AxeNStax → Forgesworn proposal)

**Status:** PROPOSAL / design-only, 2026-06-04. AxeNStax-originated, **Forgesworn-owned** per the
standing boundary (`signet-plans/MESSAGE-FROM-AXENSTAX.md` convention: AxeNStax proposes with
use-case + wire shape; Forgesworn authors the spec/NIP). Not built. Nothing on `main` changes from
this doc.

**One-line:** the **public sibling** of [Stash](https://github.com/forgesworn/stash) — signed,
plaintext, content-addressed items a persona publishes for anyone to discover and adopt. Stash is
your private vault; **Beacon is your public shelf.**

---

## Why this exists (the decision trail, so it isn't re-litigated)

AxeNStax needs public content distribution: official + community **mods, scenarios, and — via the
Workshop (`docs/foundations/2026-06-04-the-workshop-community-redesign.md`) — community visual
redesigns** that any player can browse and adopt. We prototyped this AxeNStax-locally as "open
stash" (`game/engine/src/open_stash.rs`, kind 30820, `tools/sites/game/static/openstash.js`).

Studying the Forgesworn data primitives clarified that this is **its own concern**, not a mode of
anything that exists:

| Use case | Primitive | Encryption | Status |
|---|---|---|---|
| Private save / backup / sync | **Stash** | encrypt-to-self (NIP-44) | ✅ built, AxeNStax consumes it |
| Share with an **audience** (family, subscribers, paywall) | **Dominion** | encrypt-to-others (epoch keys) | ✅ built |
| **Local** on-device | **Locket** | n/a | 🔲 reserved |
| **Fully public** (anyone fetches, in the clear) | **— nothing —** | **none (plaintext, signed)** | ❌ the gap → **Beacon** |

- It is **not Stash** — Stash is defined by encrypt-to-self / zero-knowledge; a plaintext-public
  mode would destroy that headline guarantee. (Confirmed by reading `forgesworn/stash` — its whole
  value is "no server can read your data.")
- It is **not Dominion** — Dominion's value is *encryption + scalable key distribution + revocation*
  for a controlled audience. Fully-public plaintext publishing needs none of that machinery; using
  Dominion for it is the wrong shape.

So Beacon is a distinct, fourth member of the family.

## Why it is *light* (NIP-first, not a heavyweight SDK)

Stash and Dominion are heavyweight primitives because they solve genuinely hard problems
(zero-knowledge manifests; epoch content-keys + revocation). **Public publishing has no hard
problem** — strip it down and it is:

- upload a **plaintext** blob to Blossom (content-addressed sha256 — a one-liner with
  `blossom-client-sdk`),
- sign a **manifest** event listing your public items (a plain Nostr event),
- let others **follow** you and **fetch by hash**.

That is Nostr working as designed. So the genuinely valuable, interoperable artifact is **a NIP that
standardises the public-collection manifest shape + content-type conventions** — *not* a Stash-scale
code library. A thin helper can follow once a second consumer proves the abstraction; until then
AxeNStax's `open_stash` is the de-facto reference implementation. This keeps us on the right side of
the AxeNStax↔Forgesworn boundary (share the *standard*, don't promote AxeNStax code to "shared
infra" before reuse is real).

## Prior art — align, don't fragment

Public blob-collections already exist in the ecosystem; Beacon should **reuse standard kinds**, not
mint a blind new one (the way AxeNStax's placeholder kind 30820 currently does):

- **Blossom Drive** — public folder manifests at **kind 30563** (a real prior kind; the app is
  abandoned, but the kind is the closest existing standard).
- **Bloom** ([Letdown2491/bloom](https://github.com/Letdown2491/bloom), MIT) — does public folders
  via **NIP-51 lists** + **NIP-94 file metadata** + NIP-19 refs, with NIP-44 only on the *private*
  side. Living proof the standard-NIP composition works for exactly this.
- **Blossom** (BUD specs / NIP-B7) — the content-addressed blob layer, with maintained SDKs
  (`blossom-client-sdk` JS, `nostr-blossom` Rust). **Beacon sits on Blossom; it does not reinvent
  storage.**

**For Forgesworn to decide (you own the NIP):** does a Beacon manifest reuse **NIP-51 list**
semantics, align with **kind 30563**, or warrant a dedicated kind via `nip-drafts`? Recommendation:
start from NIP-51 + NIP-94 composition (Bloom's proven path) and only mint a dedicated kind if those
genuinely don't fit.

## Proposed shape (sketch — Forgesworn finalises the wire)

All standard Nostr + Blossom, no new cryptography:

1. **Items are plaintext blobs on Blossom**, addressed by sha256. No encryption (it's public).
2. **A signed, parameterised-replaceable manifest** per app namespace lists the persona's published
   items — each `{ name, blobHash, contentType, size, updated }` (+ optional free metadata).
   `contentType` is **free-form** (`override-set`, `scenario`, `mod`, `texture-pack`, …) so any app
   reuses Beacon unchanged.
3. **Discovery = follow.** Resolve a publisher's npub → read their Beacon manifest(s). "Official"
   sources are well-known npubs (e.g. AxeNStax's). A NIP-51 follow-list of Beacon publishers is the
   natural discovery layer.
4. **Adopt = fetch.** GET the item's blob by hash from Blossom; the consuming app interprets it by
   `contentType`. No keys, no decryption.

### Symmetry with Stash (the developer mental model)

```
Stash   : save(app, name, bytes)    → encrypted-to-self, private, cross-device
Beacon  : publish(app, name, bytes) → signed plaintext, public, world-readable
          follow(npub) / list(npub) / fetch(ref)
```

Distinct **verbs** (`save` vs `publish`) on purpose — see safety, below.

## Safety / posture (matters for a kids' platform)

- **Publishing is a deliberate, differently-named act.** Beacon content is plaintext, signed under
  your name, public, and effectively permanent (content-addressed + mirrored). `publish()` must be a
  separate, explicit action from Stash's private `save()` — a guardrail against accidental public
  exposure (especially for children). The NIP should note this consumer-UX expectation.
- **Moderation / IP stays with the consuming app + relay/Blossom operators**, not the primitive —
  same as all public Nostr content. Beacon is neutral transport.
- **No private data path.** Beacon never encrypts and never carries anything a user would want
  private; that is Stash's job. The split keeps each primitive's security story clean and auditable.

## What AxeNStax consumes (the reference use case)

- **`open_stash` → Beacon.** AxeNStax's local open-stash (kind 30820, plaintext signed manifest +
  Blossom blobs + followed-npubs) is already Beacon-shaped. When the Beacon NIP lands, AxeNStax
  migrates `open_stash` onto the standard kind and contributes back as the first reference consumer.
- **Workshop Phase D** (`docs/foundations/2026-06-04-the-workshop-community-redesign.md`): *publish a
  community redesign* = Beacon publish; *adopt someone's redesign* = Beacon fetch; *private backup of
  your own override-set* = **Stash** (not Beacon). This doc supersedes the Workshop spec's loose
  "open-stash" references — private→Stash, public→Beacon.

## Open questions for Forgesworn (spec owner)

1. **Manifest kind** — NIP-51 list reuse vs align-with-30563 vs dedicated `nip-drafts` kind. (Lean:
   NIP-51 + NIP-94 composition first.)
2. **Per-app namespacing** — the `d`-tag convention so one persona's many apps' Beacons coexist (this
   is exactly the hardcoded `"axenstax-open-stash"` d-tag that must become a parameter).
3. **Discovery model** — NIP-51 follow-list of Beacon publishers, per-app or global; how "official"
   sources are marked.
4. **Helper now or later** — ship a thin `@forgesworn/beacon` publish/follow/fetch helper now, or
   keep AxeNStax `open_stash` as the reference implementation until a 2nd consumer appears. (Lean:
   later — NIP first.)
5. **Name** — "Beacon" (AxeNStax's pick: the mechanic *is* "send it out for anyone to pick up";
   pairs against Stash without implying a mode of it; generic word, no franchise/trademark
   collision). Forgesworn's call to adopt or rename.

## Boundary note

Per `MESSAGE-FROM-AXENSTAX.md`: this is a general-purpose primitive proposal (every Nostr app that
wants public, user-owned content distribution benefits — mods, skins, documents, datasets), **not**
AxeNStax-specific work repackaged. AxeNStax proposes + drives the first use case; **Forgesworn owns
the NIP + any helper**, reconciling our placeholder kind 30820 into the standard. Happy to PR the
reference consumer once the wire is settled.
