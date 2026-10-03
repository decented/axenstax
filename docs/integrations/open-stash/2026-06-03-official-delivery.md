# Official open-stash delivery + booth cache — build notes + publish runbook

**Date:** 2026-06-03 · **Goal:** btc-prague-demo Goal 5 · **Status:** delivery code built + unit-tested; **the live official PUBLISH is an OWNER step** (this doc). One UX seam (in-game menu render) is flagged as the remaining integration.

Delivers Hash Dash + Satori Rush as **official downloadable mods from the AxeNStax open-stash**, with a **local cache pre-seeded from the embedded def artifacts** so the booth works with **zero network**. Builds on Goal 2 (open-stash), Goal 1 (the ScenarioDef loader), and Goals 3/4 (the def artifacts).

## What was built (autonomous, `check.sh` green)

| Piece | Where | Tested |
|---|---|---|
| **Pre-seeded booth cache** — the embedded Hash Dash + Satori Rush defs (`include_bytes!`), playable with zero network | `official_content::seeded_official_challenges` | ✅ |
| **Cache + live merge** — fetched open-stash content updates/augments the seed; the seed is always the fallback; corrupt/unparseable blobs are skipped (never clobber the seed); non-scenario items ignored | `OfficialContentCache::{ingest_manifest, challenges, challenge}` | ✅ (8 tests) |
| **Default-follow the AxeNStax npub** | `default_followed` = Goal 2's `FollowedNpubs::with_official` | ✅ |
| **Save-as-own** — the official def is a read-only template (`OfficialChallenge::def` re-parses; the seed is stable); starting + saving a challenge writes the player's OWN world | `OfficialChallenge::def` + property test | ✅ |
| **Publish inputs** — the official defs as `(name, "scenario", bytes)` open-stash items | `official_open_stash_items` | ✅ |
| **Live fetch (WASM)** — refresh the cache from the followed npubs' open-stash (take-home/update path) | `refresh_official_content` (cfg wasm) | owner live-verify |
| **Publish (WASM)** — publish the official defs to the signed-in identity's open-stash | `publish_official_content` (cfg wasm) | owner live-verify |

**Booth posture:** the challenges are launchable **today** with zero network — the defs are compiled in and start via **`/scenario hash-dash`** and **`/scenario satori-rush`** (Goals 3/4). The live open-stash fetch is the *update / take-home* path layered on top; the cache always falls back to the embedded seed.

## ⚠️ OWNER step — publish the official content (live)

The official defs must be published to the **AxeNStax npub's** open-stash. This needs the real AxeNStax signing key, so it's an owner step.

1. **Set the official npub.** `open_stash::OFFICIAL_AXENSTAX_PUBKEY_PLACEHOLDER` is 64 hex zeros. Replace it (via the `official_axenstax_pubkey()` seam) with the real AxeNStax pubkey, so clients `default_followed()` the right identity. Rebuild.
2. **Stand up a public Blossom + the relay** (same prerequisites as Goal 2 — see `2026-06-03-open-stash-primitive.md`): `BLOSSOM_PUBLIC_URL` set, `wss://relay.trotters.cc` up.
3. **Publish, signed in as AxeNStax.** Start the game site, sign in with the **AxeNStax** Signet identity, and from the DevTools console call the publish export:
   ```js
   // The engine exposes publish_official_content() (wasm). If not bound to a
   // window function in the booth build, trigger it via the dev path that calls
   // official_content::publish_official_content().
   ```
   This PUTs the embedded Hash Dash + Satori Rush defs (plaintext) to Blossom and publishes the AxeNStax open-stash manifest (kind-30820, d-tag `axenstax-open-stash`, two `scenario` items).
4. **Verify from a clean client.** Sign in as any other identity (or anonymously — reads need no signer), `default_followed()` already follows AxeNStax, and `refresh_official_content` should list + download both challenges. Confirm they match the embedded seed.

## Remaining UX integration (low-risk; needs the running app to verify)

The **in-game menu "Official Challenges" surfacing** — a section in `menu.rs::draw_main_menu` that lists `OfficialContentCache::challenges()` and, on click, **creates a world and starts the scenario** — is the one piece left. It's deliberately not wired here because the create-world→start-scenario flow is multi-frame + ordering-sensitive (and the visual UX needs the running app to verify), so it belongs to a supervised/playtest pass rather than an unattended build. The seam is ready:

- **Data:** `official_content::OfficialContentCache::challenges()` → the list to render; `::challenge(name)` → the chosen def.
- **Recommended wiring:** mirror the Create-dialog flow (`menu.rs:603-632`): on a challenge button, build `WorldMeta::new(display_name)`, set `meta.scenario_def = Some(def.to_json())` **and** `meta.seed = def.arena_seed.unwrap_or_else(save::gen_random_seed)`, `save_world_meta`, and return `MenuAction::LoadWorld(folder, pref)`. Setting the seed here is the **only** correct place to honour `arena_seed` (a fixed, fair Hash Dash arena): it's a world-*creation* property — `/scenario` in an existing world can't regenerate terrain because `initial_load` restores the save. The Goal 4 load-path restore (`game_loop.rs`, after the stat-seed) already reconstructs the scenario from `meta.scenario_def` on load. **Gap to close in that pass:** provision the def's `kit` on a *fresh* start (`total_ticks == 0`) by calling `commands::builtins::give::resolve_item(&kit_item.name, kit_item.count)` for each `def.kit` entry (the same resolver the `/scenario` handler uses) — Satori Rush has an empty kit (works as-is); Hash Dash needs its pickaxe/axe/bread.
- **Until then:** `/scenario hash-dash` / `/scenario satori-rush` are the booth launch path (kit provisioning included), and the cache/fetch/publish are all in place.

## Source of truth
`docs/superpowers/specs/2026-06-03-conference-demo-plan.md` §2b (official-content model); Goal 2 primitive (`2026-06-03-open-stash-primitive.md`).
