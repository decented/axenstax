# Native Build & Distribution — Overarching Strategy

**Status:** Strategy, not a spec (captured 2026-06-06; **revised same day after an
OSS survey** — see "Existing open source to draw in"). High-level direction +
staging; specs/plans fork off this when a stage is scheduled. Estimates are
**agent-time**, calibrated against observed throughput (≈239 commits / ~53k
insertions in the prior 8 days, incl. the Beacon Nostr+Blossom primitive in ~1 day).

## The goal

Turn the native engine from **raw binaries** (chmod-and-pray) into **download-and-
play, semi-production-ready packages** on Windows, macOS and Linux — and, in time,
give native the same features the web has (cloud save, then multiplayer).

## Why native at all (the payoff, for context)

Native is **additive, for specific players** — the PWA stays the easy front door for
the casual/Chromebook tester. Native wins for: **performance & scale** (FPS, render
distance, bigger worlds, no WASM memory ceiling, full threading), **couch co-op /
split-screen** (multiple controllers, one screen — already built), **LAN & self-
hosted play** (raw UDP networking the browser can't do), **controllers**, **offline /
real-app feel** (no browser-compat gate), and it's the **only road to the
console/handheld vision**.

## Guiding principles

1. **PWA-first stays.** Native is the power-user / couch-coop / self-host / console
   track, not a replacement.
2. **Draw in mature OSS, don't rebuild — then contribute upstream.** Almost
   everything here already exists as healthy open source (see the survey below). We
   **consume** it, write **thin glue**, and **upstream** anything we extend — staying
   *in* the ecosystem rather than forking off it.
3. **Reusable primitives, not bespoke.** Whatever glue *is* ours and isn't AxeNStax
   gameplay (signing config, save-sync orchestration, packaging CI) is built as a
   **Forgesworn/Decented reusable package**, never hard-coded to AxeNStax. (Standing
   shared-infra rule + the Signet boundary.)
4. **Self-distribute.** Steam **bans crypto / earn-real-value games**, so it's out —
   fine: own site + **itch.io** + **Flathub** fits the open-source, self-sovereign
   ethos anyway. No 30% cut, no gatekeeper.
5. **Two quality tiers:** unsigned-alpha (free, some friction) → signed-polished
   (Apple/Windows accounts, zero warnings).
6. **No single format covers all three OSes** (AppImage is Linux-only) — but **one
   tool builds all three**: `cargo-packager`.

## Existing open source to draw in (the survey)

The "build these primitives" plan mostly collapses into "compose these libraries":

| Need | Draw in | Maturity | Our net-new |
|---|---|---|---|
| Native NIP-46 signer (keystone) | **`nostr-connect`** / **`nostr-signer`** (rust-nostr) | maintained (v0.44+) | a thin Signet-bunker config wrapper |
| NIP-44 encryption, relay, keys, events | **`nostr-sdk`** (rust-nostr) | mature | — (consume) |
| Blossom blob client | **`blossom-rs`** (full BUD-01 client + auth; embeddable server) — or `nostr-blossom` (alpha) | blossom-rs solid; nostr-blossom alpha | — (consume) |
| Save-sync (encrypted blob + Nostr manifest) | *composes* nostr-sdk + blossom-rs + nostr-connect | n/a | a thin **`stash-rs`** orchestration |
| Installers (Win/Mac/Linux) + auto-update | **`cargo-packager`** (CrabNebula/Tauri) — `.dmg/.app`, `.msi/NSIS`, AppImage/`.deb`, + updater | production-grade | a shared config + CI |

So `signet-nip46-client` ≈ "configure `nostr-connect` for the Signet bunker flow"
(Signet uses NIP-46, so this is **Signet-boundary clean** — consuming generic NIP-46).
**Bonus:** rust-nostr also ships **NWC (Nostr Wallet Connect)** — the Rust home for the
in-game Lightning wallet too. Drawing it in serves more than cloud save.

**Caveats:** maturity varies (pin versions; `nostr-blossom` is alpha) and every one of
these is **native-only — `cfg(not(wasm32))`-gate them** so the WASM bundle never sees them.

## Two tracks

### Track A — Packaging & distribution (binary → playable)  *(actionable now)*

Backbone: **`cargo-packager`** — from the one Rust project it packages
Windows/macOS/Linux installers + an auto-updater, in CI. (`cargo-dist` was the first
pick but is CLI-tool-oriented; `cargo-packager` is app/GUI-oriented and does dmg + msi
+ AppImage natively — the better fit for a game.) Target formats:

| OS | Format | Notes |
|---|---|---|
| Windows | `.msi` (WiX) or NSIS `.exe` | unsigned → SmartScreen warning until code-signed |
| Linux | **AppImage** (single file) + `.deb` | Flatpak/Flathub later for Steam Deck; GPU drivers stay host-side |
| macOS | `.dmg` + `.app` | Apple Developer ($99/yr) to sign + notarize; **else Gatekeeper *blocks* downloaded apps** (Sequoia+ removed the easy bypass) — see A2 |

**Stages**
- **A0 — `cargo-packager`, unsigned.** Tagged release → the three installers + a
  download page, from CI. Turns "a binary" into "download-and-run." *(small)*
- **A1 — Wire the download page** into the `.org` self-host area; AppImage as the
  friction-free Linux single-file (native to packager).
- **A2 — Sign + notarize** (Apple Developer + a Windows cert / Azure Trusted Signing)
  → zero warnings. *(needs accounts/$)* **Severity is asymmetric — A2 is not uniform
  polish:**
  - **macOS = hard blocker, not a warning.** An unsigned/un-notarized `.dmg`/`.app`
    downloaded from the web carries `com.apple.quarantine`, and Gatekeeper **blocks
    it** ("app is damaged" / "developer cannot be verified"). On **macOS Sequoia (15)+**
    Apple **removed the Control-click → Open bypass** — the user must dig into System
    Settings → Privacy & Security → "Open Anyway," which most testers read as broken/
    malware. **Notarization requires an Apple Developer account** (a self-signed cert
    doesn't satisfy Gatekeeper), so the **$99/yr is the difference between
    "downloadable" and "dead on arrival" for non-technical macOS users** — not optional
    once you have real macOS testers.
  - **Windows = friction, not a block.** Unsigned → SmartScreen "protected your PC"
    dialog; user clicks **More info → Run anyway** and it installs. A cert removes the
    warning (EV / Azure Trusted Signing = instant reputation; plain OV still warns until
    download-reputation accrues). Works without paying; just scary.
  - **Linux = free and clean.** No signing gate — AppImage needs only `chmod +x`. No
    $-account dependency.
  - **Sequencing takeaway:** if testers skew Linux/Windows, **A2 can wait**; the moment
    meaningful macOS testers appear, the Apple Developer account stops being optional.
- **A3 — Channels + updates:** Flathub (Steam Deck), itch.io, `cargo-packager-updater`.

### Track B — Native feature parity (sign-in → cloud save → multiplayer)  *(gated)*

Native has **no Nostr sign-in / signer**, which blocks cloud save *and* multiplayer
auth. The expensive part is the signer — but it's largely **`nostr-connect`**, not
from-scratch.

**Stages**
- **B0 — Eval spike (de-risk first):** in a throwaway, confirm (1) `nostr-connect`
  pairs with a **real Signet bunker** and signs; (2) `blossom-rs` round-trips a blob to
  **Primal's Blossom**; (3) `cargo-packager` builds the 3 installers AND the native
  crates `cfg`-gate cleanly so the **WASM bundle still builds**. If these pass, the
  estimate drops further — we're gluing, not authoring.
- **B1 — Native sign-in** (`nostr-connect` + Signet bunker — keeps the "key on your
  phone" model). The keystone; also unblocks native multiplayer.
- **B2 — Native cloud save** (`stash-rs` glue over nostr-sdk + blossom-rs). ~1–2
  agent-days on top of B1; mirrors the web's Stash/Beacon.
- **B3 — Native multiplayer auth** (same signer). Later, with the multiplayer fleet.

Rough agent-time: **Track B ≈ a week** to solo-verifiable (less than the earlier
estimate now that the signer is mostly OSS), then a single **real-device test** (your
phone + Signet) that agent speed can't compress.

## What we build vs draw in (the cross-game-lift map)

| Thing | Ours? | Approach |
|---|---|---|
| Installers + updater + CI | shared | **draw in `cargo-packager`** + a reusable "ship a Decented game" config/CI |
| OS-aware download page | shared | thin shared template |
| Native NIP-46 signer | shared | **draw in `nostr-connect`** + a thin Signet-bunker wrapper (Forgesworn) |
| Blossom client + crypto/relay | shared | **draw in `blossom-rs` + `nostr-sdk`** |
| Save-sync orchestration | shared | thin **`stash-rs`** (Forgesworn primitive) composing the above |
| `save.rs` integration, engine, content | AxeNStax | the only bespoke code |

Rule: if it isn't *this game's gameplay*, draw in OSS first → thin reusable glue →
contribute upstream. Nothing native-bespoke that another Decented game can't reuse.

## Execution split — what lives where, what touches the engine

Three buckets by *where it lives* and *whether it touches the engine*. Buckets 1 + 2
run **in parallel, now, with no engine edits and no collision** with active engine
work; bucket 3 is the small, deliberately-last integration.

### 1. Forgesworn primitives — reusable, cross-game (live in `forgesworn/`)

The thin glue we author over the drawn-in OSS; lifts to every Decented game.
- **Signet-bunker signer** — a thin wrapper over **`nostr-connect`** configured for the
  Signet NIP-46 bunker flow. (Signet-boundary clean — consumes generic NIP-46.)
- **`stash-rs`** — save-sync orchestration: encrypted blob (**`blossom-rs`**) + signed
  Nostr manifest (**`nostr-sdk`**) + the signer. The Rust sibling of `@forgesworn/stash`.
- *(Drawn-in OSS these consume — not ours to author, but upstream-contribute to:
  `nostr-sdk` / `nostr-connect`, `blossom-rs`, `cargo-packager`.)*
- *(Shared game-ops, reusable but not crypto: a "ship a Decented game" `cargo-packager`
  config + CI **template** — a Forgesworn/Decented shared kit.)*

### 2. AxeNStax side, no engine touch — buildable now, zero collision

External to the engine *source* — config, CI, website, throwaway harness:
- **`cargo-packager` config** for this game (a standalone config file — not the engine `Cargo.toml`).
- **Release / packaging CI workflow** (GitHub Actions).
- **OS-aware `/download` page** serving the three installers (alongside the build-from-source steps).
- **Packaging runbook / docs.**
- **B0 eval spike** — a throwaway crate exercising `nostr-connect` + `blossom-rs` +
  `nostr-sdk` (compile + a real Blossom round-trip to Primal). De-risks bucket 1.
- A non-engine **test harness** that consumes the bucket-1 crates standalone.

### 3. Engine integration — the final, engine-touching bit (coordinate + needs you)

Small, last, done with the engine owner:
- Add the bucket-1 crates as **deps in the engine `Cargo.toml`** + **`cfg(not(wasm32))`-gate**
  them so the WASM bundle never sees them (dual-target).
- Wire **`stash-rs` into `save.rs`** (the cloud-save call sites).
- **Native sign-in UX** in-engine (the Signet bunker pairing flow).
- In-engine **packaging metadata** if any (icon, app name).
- **Real-device verification** — pair a live Signet bunker on a phone (only the owner can).

**Flow:** build 1 + 2 in parallel now → bucket 3 is the small final integration once
1 + 2 are proven.

## Sequencing

- **Track A can start now** — independent of native sign-in; makes the *existing*
  binaries playable. Smallest, highest immediate value.
- **Track B is gated** on the eval spike (B0) then the signer (B1); co-delivers with
  native multiplayer.
- Signing/notarization (A2) waits on the Apple/Windows accounts; everything before it
  is free.

## Channels (decided)

Own site (the `.org` self-host area) + **itch.io** + **Flathub**. **Not Steam**
(crypto ban). Fits the self-hostable, open-source posture.

## OSS references

rust-nostr (`nostr-sdk` / `nostr-connect` / `nostr-signer`), `blossom-rs` /
`nostr-blossom`, Blossom spec (hzrd149) + awesome-blossom, `cargo-packager` (CrabNebula).
