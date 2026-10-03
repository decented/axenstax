# Per-Persona Encrypted Save Vault — architecture note

**Status:** ARCHITECTURE / THINKING DOC, 2026-06-01. No build commitment. Captures a design that emerged while scoping AxeNStax "cloud save" (Phase 5) and discovering the real requirement is a **cross-game Forgesworn primitive**, not an AxeNStax feature.

**Origin:** Brainstorm 2026-06-01 (Staxolottle). The ask started as "back up my world to the cloud," sharpened to "log into AxeNStax on any machine, pick my persona, get my worlds," and then — on the observation that *a persona saves many things across many games* (an AxeNStax world, a racing track, a skin in some other game) — generalised to a per-persona encrypted save vault that any Forgesworn game writes into.

---

## TL;DR

A **persona** (a Heartwood-derived, unlinkable Nostr identity the *user* chooses — not one-per-game) gets a **private encrypted save vault**:

- Each saved item (a world, a track, a skin) is **NIP-44 encrypted-to-self** with the persona's key and stored as **one blob in Blossom** (content-addressed by ciphertext hash). Blossom is dumb storage — it never sees plaintext and doesn't know whose blob is whose.
- A **manifest** indexes everything the persona owns, namespaced by app: `persona-npub → { app → [{kind, name, blobHash, updated, size}] }`.
- **Restore-anywhere:** log in as the persona on any device → read its manifest → fetch + decrypt the blobs you want.

It's **zero-knowledge** (the server/relay/Blossom only ever hold ciphertext + hashes), **cross-game** (apps share the vault, each under its own namespace), and **identity-portable** (the key is the persona, available on any device via the signer). AxeNStax is the **first consumer**, contributing `app: "axenstax"` world entries.

---

## Why this is a Forgesworn primitive, not an AxeNStax feature

The decisive realisation: a persona saves **many things across many games**. If AxeNStax built its own world-manifest, the racing game and the skin tool would each reinvent the same wheel, and a persona's stuff would be scattered across incompatible per-app schemes. The thing that makes "log in anywhere, get *your stuff*" work is a **single index per persona that spans apps** — which is inherently cross-game.

This matches the shared-infra strategy (`project_shared_infra_strategy`): build primitives that lift to other games; don't hard-code AxeNStax assumptions into shared infra. It also respects the Signet-boundary rule (`feedback_signet_boundary`): the primitive is generic (encrypt-to-self + indexed blob storage), not AxeNStax work repackaged.

**Ownership:** the vault primitive should live in Forgesworn (a new repo/SDK — see "Where it lives"), with AxeNStax depending on it. It sits in the family alongside Heartwood (keys), Blossom (blobs), and Dominion (the *sharing* counterpart — see below).

---

## The layer cake

```
┌─ Consumers ─────────────────────────────────────────────────┐
│  AxeNStax (worlds)   racing game (tracks)   skin tool (skins) │  ← each writes under its own app namespace
└──────────────────────────┬──────────────────────────────────┘
                           │  save(app, kind, name, bytes) / list(persona) / restore(blobHash)
┌─ Vault primitive (NEW, Forgesworn) ─────────────────────────┐
│  • orchestration: encrypt → put → record in manifest         │
│  • manifest: per-persona, per-app index (see schema below)   │
└───────┬───────────────────────────────┬─────────────────────┘
        │ encrypt-to-self               │ blob put/get
┌─ NIP-44 (signer) ─────────┐   ┌─ Blossom ──────────────────┐
│ session.signer.nip44       │   │ dumb content-addressed      │
│ .encrypt/.decrypt          │   │ ciphertext blob store       │
│ (needs a bunker session)   │   │ (built + hardened, AxeNStax)│
└───────┬────────────────────┘   └─────────────────────────────┘
┌─ Heartwood / Signet ──────────────────────────────────────────┐
│ one mnemonic → unlimited unlinkable personas; key never leaves │
│ device; NIP-46 remote signing; user CHOOSES the persona        │
└────────────────────────────────────────────────────────────────┘
```

### 1. Persona (Heartwood / Signet)
A persona is a **user-chosen, Heartwood-derived identity** (`nsec-tree` under the hood: one mnemonic → unlimited unlinkable personas — "work, personal, anon"). **Not per-game** — the user decides whether their AxeNStax worlds and their racing tracks live under the same persona (linked, convenient) or separate personas (unlinkable). The key never leaves the signer; the app asks the signer to encrypt/decrypt/sign over NIP-46.

### 2. Encryption (NIP-44 encrypt-to-self)
Each blob is encrypted with the persona's key to *itself* (ECDH against its own pubkey). Only that persona's key decrypts it. **This is already in the `signet-login` signer interface** (`SignetSigner.nip44?.{encrypt,decrypt}`, gated by `capabilities.hasNip44`) — *but only on a signer that has an ongoing channel*: the **bunker** (NIP-46) signer has it; the **EphemeralSigner** from the redirect/QR-only flow does **not** (`canSignEvents:false`, no nip44). See "Prerequisite" below.

### 3. Storage (Blossom)
Already built + hardened (`infra/blossom/`, `POST /worlds/upload` / `GET /worlds/download/{hash}` proxy). Stores **ciphertext** blobs, content-addressed by hash. Zero-knowledge holds because it only ever sees ciphertext. The existing AxeNStax server-proxy + server-key Blossom auth stay as the **bridge transport** — they handle only ciphertext + hashes, so they don't break zero-knowledge.

### 4. The manifest (the new index)
The record of "where is all my stuff." **Destination design: a persona-signed, self-encrypted, parameterized-replaceable Nostr event per app** (rides relays, no custom server — the same posture as Dominion):

- `kind`: a parameterized-replaceable manifest kind (3xxxx range; exact number TBD via a NIP draft).
- `pubkey`: the persona.
- `d` tag: the app id (`axenstax`, `speedgame`, `skinmaker`). One replaceable event per (persona, app) → apps stay decoupled; each writes only its own `d`.
- `content`: NIP-44 encrypt-to-self of the item list:
  ```json
  [
    {"kind":"world","name":"Jungle Base","blobHash":"<sha256-of-ciphertext>","updated":1717200000,"size":51234},
    {"kind":"world","name":"Sky City","blobHash":"…","updated":1717100000,"size":80012}
  ]
  ```
- **Read across games:** query one relay for all manifest-kind events by the persona pubkey → one event per app the persona uses. **AxeNStax's worlds** = the `d:axenstax` event, decrypted.

**Bridge design (interim, AxeNStax-only):** a server-side `persona-npub → [worlds]` table in `app.py` (operates on hashes only, never plaintext). Lets AxeNStax ship cross-device before the relay-event manifest + cross-game story are built. The bridge→destination swap changes only the manifest store, not the encrypt/blob layers.

### 5. Consumers
Each game calls the vault: `save(app, kind, name, bytes)`, `list(persona)`, `restore(blobHash)`. AxeNStax wires its existing `save_world`/`load_world` WASM path through it under `app:"axenstax"`. Per-world opt-in (`Save online` toggle, default OFF) is an AxeNStax-side policy on *whether* to call `save`.

---

## Relationship to Dominion (and why this is NOT Dominion)

`forgesworn/dominion` is the **sharing** counterpart: encrypt content *to an audience* (tiers, revocable, scales to 1000s of recipients via epoch content-keys + key-shares). This vault is **encrypt-to-self, indexed** — no recipients, no revocation, no audience. They share the NIP-44 substrate and the word "vault," but solve opposite problems (share-with-others vs keep-for-myself). The vault could *later* use Dominion to share a saved world with a friend, but that's a separate, future capability.

---

## Prerequisites / what's blocked

| # | Need | Layer | Status |
|---|------|-------|--------|
| 1 | **Retain a bunker signer at login** so `signer.nip44.encrypt/decrypt` is callable (AxeNStax currently discards the signer, keeping only the pubkey — `auth.js` `runFreshAuth`). | AxeNStax-side | **Buildable now** — no upstream change. The bunker path already exists (`lobby.js` persists `bunkerUri`/`bunkerClientSk`). This is the next concrete step (own spec). |
| 2 | **User-chosen persona** surfaced to the app (which Heartwood persona am I acting as?). | Heartwood / signet-login | Partly reserved scope ("Per-game persona derivation | Heartwood RPC" in the signet-login README). Needs an upstream conversation — but note Heartwood *already* derives unlimited personas; the gap is the SDK surfacing the choice to consumers. |
| 3 | **Manifest event kind** (the parameterized-replaceable cross-app index) + a NIP draft. | Forgesworn (new) | Design work — part of the vault primitive spec. Bridge (server table) avoids needing this to ship AxeNStax cross-device. |
| 4 | **The vault SDK itself** (orchestration + manifest). | Forgesworn (new repo) | Not started — this doc is the precursor. |

**Honest read:** #1 is small and unblocked (do it next). #2/#3/#4 are real, larger, cross-repo efforts. None of this is "AxeNStax Phase 5" anymore — Phase 5 is reduced to "AxeNStax consumes the vault."

---

## Where the primitive should live

A **new Forgesworn repo** (working name e.g. `vault-kit` / `keepsake`), MIT, TypeScript SDK + a NIP draft for the manifest kind. Depends on: a NIP-44-capable signer (Heartwood/signet-login), a Blossom server, standard relays. Rationale: it's a clean primitive with no AxeNStax specifics; every Forgesworn game inherits it; it slots beside `dominion`/`heartwood`/`blossom` in the family. The exact repo/name is a Forgesworn-side decision — flagged here, not decided.

---

## What AxeNStax does in the meantime

1. **Build prerequisite #1** (retain bunker signer + expose NIP-44 to the WASM save/load path) — concrete, unblocked, useful regardless of how the vault lands. **Next artifact** (its own spec).
2. Keep the **bridge** cloud-save (server proxy + server-side manifest table, NIP-44 encryption once #1 lands) so AxeNStax gets working zero-knowledge cross-device save before the full cross-game vault exists.
3. Migrate to the vault SDK when it ships — transparent to gameplay (only the manifest store + persona-selection change).

---

## Open questions for Forgesworn

1. **Persona selection UX** — when a user logs into AxeNStax, do they pick a persona, or does the app request a default "gaming" persona? Needs Heartwood/signet-login product input.
2. **Manifest kind number** + whether it's one-event-per-app (`d`=app) or one-event-with-all-apps. (This doc assumes per-app for decoupling.)
3. **Blob auth at the destination** — bridge uses the AxeNStax server key; does the vault want player-key Blossom auth (portable blobs, fetchable from any Blossom client) or is server-proxied fetch enough? (Separate from encryption; portability vs simplicity.)
4. **Quota / fair-use** per persona across apps.

---

## Memory-rule check

- `project_shared_infra_strategy`: built as a cross-game primitive, AxeNStax as first consumer. ✅
- `feedback_signet_boundary`: the upstream asks (#2) are generic persona-selection capability, not AxeNStax-specific Signet work. ✅
- `reference_trotters_relay`: the destination manifest publishes to our own trotters relay. ✅
- `feedback_npub_only_display`: any user-facing surface shows npub/handle, never hex. ✅
- UK English throughout. ✅
