# The Workshop — community visual redesign (reskin existing assets, global overrides)

**Status:** ⚙️ **BUILT (local) 2026-06-05** — Phases **A, B, C, F shipped to `main`**; Distribution
Phases **D (share)** and **E (official-adoption bundle)** **BUILT** on branch
`feat/workshop-beacon-share` (`./check.sh`-green, 2145 engine tests), realised on the
`@forgesworn/beacon` primitive — the `open_stash` prototype is **retired** (manifest build/parse/
schnorr-verify now live in the vendored SDK; the engine keeps only thin byte-marshalling externs).
Settled decisions this build: the share blob is a **1-byte-version-prefixed bincode** `OverrideSet`
(permanent public wire format; newer-version blobs are rejected, not misparsed); Beacon kind stays
**30820**, `d`-tag = bare **`axenstax`** (SDK-derived from `app`); adopt is **whole-set
last-adopted-wins** per asset key + one derivation link; the official catalogue applies
**beneath** personal (personal wins, official is a base layer with no provenance bleed); npub is
decoded at follow-input + encoded at browse-display (new `npub.rs` + `npub-decode.js` sibling; hex
stays internal). **Owner boundary** (not built here): the live publish→adopt round-trip
(`wss://relay.trotters.cc` + Blossom, no Docker on host), the real official npub + live bake, and
the polished Share/Browse UI (the `/ws` command bridge is the built surface). What's built, local +
testable:
- **A — override layer** (`override_registry.rs`): per-asset 16×16 reskins appended to the block
  texture array (Spec 03 §3.2) + consulted at the block-mesher (`mesh.rs`) and entity-builder
  (`entity_model.rs`) seams. Empty ⇒ byte-identical.
- **B — the Workshop**: `MenuAction::EnterWorkshop` + a Lobby button; a void world preset
  (`World::generate_workshop_column`); the project model + bellows/pin state machine + a
  persistent `workshop_projects` side-table (`workshop.rs`, rides the world save).
- **C — reskin (Mode A)**: `workshop::commit_project_override` writes a project's paint into the
  registry; pinning reskins **every** instance (blocks AND mobs), live (texture rebuild + re-mesh).
- **F — reshape (Mode B)**: `workshop::capture_box_as_plan` → #18's `from_plan` bake →
  `micro_registry` shape override; single-block exact-build enforced.

**The full tactile UI is built.** The **16×16 paint-grid editor** (`workshop_painter.rs`, `/ws edit
<asset>`): six face grids seeded from the asset's *current* texture, a 16-colour palette, click/drag to
paint, Pin → global reskin. The **in-world inflated mannequins** (`build_workshop_inworld_vertices`):
each parked project renders as a scaled cube (block) / model (mob) on the void floor, reskinned, the
**mob mannequin** animating when `/ws play` is on. The **Bellows held tool** (`MaterialId::Bellows`,
provisioned in the Workshop): right-click inflates the nearest parked project (mannequin grows live),
sneak + right-click deflates. Headless shots: `--shot-painter`, `--shot-workshop`. The `/ws` command
(place / edit / paint / pump / deflate / play / pin / reshape / list / reset) drives everything.
**Only the inflate/paint/reshape _feel_** (pump rate, scale ceiling, brush ergonomics, the look) is the
**Axolittle playtest tune** — the features are built. Distribution (Phase D/E) is owner-scoped-out.

---

**Original design status (kept for reference):** DRAFTED 2026-06-04 — design/spec only. This is the
**authoring front-end + community-polish pipeline** that two existing pieces are the *backend*
for: (1) #18's static micro-model bake (`docs/foundations/2026-06-03-build-big-micro-models.md`,
**shipped**, flower playtest open) and (2) the resource-pack / texture-override path
(`docs/spec/03-rendering.md` §3.2–3.3, **spec-only, no loader built**). It picks up #18's
**explicitly-deferred** "in-engine creator UI" and re-scopes its deferred "do-ocracy curation"
into a concrete, **owner-curated v1**. Sequenced **after #18's flower playtest signs off**
(Mode B reuses that bake; Mode A is independent and can lead).

**Strategic objective (owner, 2026-06-04):** *outsource the visual redesign and polish of the
game to the community.* Alpha testers don't only test — they **fix what looks rough by
redesigning the things that already exist**. **v1 is redesigning EXISTING assets, not creating
new ones.** "Once the community is used to redesigning + reskinning what's there, then we think
about creating new items" — that next phase is **parked** ("a good problem to have").

**Branches (when built, not now):** fork off `main` → `feat/workshop-override-layer` (Phase A)
→ `feat/workshop-space-bellows` (Phase B) → `feat/workshop-face-painter` (Phase C) →
`feat/workshop-stash-share` (Phase D) → `feat/workshop-adoption` (Phase E) →
`feat/workshop-reshape` (Phase F). Each phase is independently `check.sh`-gated; the **look**
is a playtest-boundary check the owner/Axolittle runs.

---

## TL;DR

A blank creative space — **The Workshop** — that players enter to **redesign assets that already
exist**. The core gesture: place a real block (or a stack, or a mob "mannequin"), **pump it up
with a bellows** to work on it ("blow up like a photo/bubble, *not* explode"), edit it, then
**put a pin in it** to deflate back to size — the pin is the *commit/save*. The Workshop is a
**persistent, multi-project workspace**: inflated projects are real, saved objects you can leave
half-finished and return to across sessions, with **as many on the go as you like** — pinning is
*finishing one*, on your own schedule (not a "complete it now" gate).

Two authoring modes, **both producing a global appearance override of an existing asset**:

- **Mode A — re-paper (reskin).** Paint the asset's faces at **16×16** (one sub-voxel ↔ one
  texel). Works on **blocks *and* mobs** — because mobs are textured *exactly like blocks*
  (per-face 16×16 layers in the block texture array), the **same face-painter reskins both**.
  **Reskin leads.**
- **Mode B — rebuild as microblock (reshape).** Rebuild an asset's *form* in a 16³ sub-voxel
  grid via #18's bake (flat billboard flower → 3D flower). **Later phase**; reuses the shipped
  micro-model pipeline.

**The decision that makes v1 small:** because you only ever redesign things that *already exist*,
every output is **bound to the asset it replaces** (FLOWER → your flower, COW → your cow). So:

- There is **no anonymous new item** needing an inventory home → **no "place it from inventory"
  problem** (that's the *create-new* phase, parked).
- Leaving the Workshop = **saving an override that auto-applies everywhere.** You don't place
  your flower; you redesign *the* flower and every flower updates.
- It is **purely visual** → **multiplayer-safe** (behaviour never changes; only the look), so it
  sidesteps server authority / anti-cheat entirely.

**The load-bearing new infra** is a **personal override registry** (`asset_type → authored
appearance`) applied at render time — i.e. the resource-pack/texture-override path
(Spec 03 §3.2–3.3) made real, scoped to per-asset overrides. Stash stores + shares override sets;
a lightweight **personal → open-stash → owner-curated-official** adoption pipeline turns one
tester's redesign into *the game's* art. Community voting / "asset-show" governance is **deferred**.

---

## Context pointers (build-state, audited 2026-06-04)

Line numbers are from the #18 doc + a 2026-06-04 code audit; treat as anchors, re-verify on build.

**Already BUILT (reused):**
- **Capture + "touching = joined".** `plan::capture` / `capture_connected_volume` /
  `flood_fill_volume` (`plan.rs:487-610`) — 6-connected flood-fill grabs an arbitrary multi-block
  volume; face-adjacency *is* the join rule, air breaks it. `PlanData { width, depth, height,
  cells: Vec<CapturedCell{rx,ry,rz,block_id}> }` holds the full 3D volume. **No capture change.**
- **Micro-model bake (single-block, exact-build).** `micro_model.rs::from_plan` + `bake_micro_model`
  + `MicroModelRegistry` (`block_id → BakedMicroModel`) — **shipped 2026-06-03** (#18), flowers
  live. `from_plan` **refuses** sources larger than one block (`width/depth/height > scale` →
  `BakeRefusal::TooLarge`). Multi-block micro-models are **NOT built** (see Open Questions).
- **Mobs are textured like blocks.** `ModelPart.tex_faces: [u32;6]` indexes the **block texture
  array**, full-face 0..1 UVs (`entity_model.rs:212-213`, `push_textured_quad` ~`:1549`). A mob
  face and a block face are the *same kind of thing*. **Player avatars use a SEPARATE 64×64 atlas
  path (`skin_uv.rs`) — do NOT route mob reskins through it.**
- **Mob motion.** Walk cycle + pivot rotation already ship (`entity_model.rs` walk swing +
  `build_part_vertices`) — mobs already animate; relevant only for the mannequin "play" preview.
- **Colour raw material.** 16 coloured wallpaper blocks (`WALLPAPER_*`, `block.rs:268-286`) + a
  face-overlay/decal path (`mesh.rs` `FaceAttachment::Wallpaper`) — the palette the painter draws
  with ("paint-with-blocks").
- **Texture resolution.** `texture_gen.rs` `const SIZE: u32 = 16` — 16×16 RGBA per layer. This is
  why the painter grid is 16×16 and the shape grid is 16³ (one micro-voxel ↔ one texel).
- **Stash transport.** `open_stash.rs` (kind-30820 manifests, blob hashing, followed-npubs) +
  `openstash.js` + `cloud.js`/`blossom.js` + `infra/blossom/` — **built but never run
  end-to-end, not wired into `/game`, and only knows about *whole worlds*** (see
  `project_cloud_save_bridge_on_main`, `project_stash_locket_save_architecture`).

**SPEC-ONLY / NOT built (this doc's new work):**
- **Resource-pack / runtime texture-override loader** — Spec 03 §3.2 (atlas rebuild: scan
  textures, build `name → layer_index`, upload array) + §3.3 (reference textures by name, engine
  resolves index at build time). **Designed, no loader exists.** This is the spine of Phase A.
- **General face-painter UI** — none today.
- **Block registry is HARDCODED** (`block.rs BlockRegistry::new`, hand-written `BlockDef` pushes).
  Adding a *new block* needs a recompile → that's the **parked** "create-new" phase, out of scope.

Spec cross-refs: Spec 03 (`docs/spec/03-rendering.md`) §3.2–3.3 (resource packs / texture
override — this doc is its first concrete consumer), §2.x (mesher), entity pipeline. Spec 02
(`docs/spec/02-world-format.md`) — block registry. Adjacent: #18
(`2026-06-03-build-big-micro-models.md`, the bake backend), #19
(`2026-06-03-dynamic-asset-authoring.md`, the *parked* mob rebuild/reanimate this doc explicitly
does **not** touch).

**Adjacent naming change (not part of this spec):** the owner also wants **crafting table →
"Workbench"** (the Workbench is the table; the Workshop is the room). That's a separate
display-name/registry change (Spec 05 surface) — captured here so the vocabulary is consistent,
**scoped out** of this build.

---

## The problem (precisely)

#18 proved the engine can be its own asset editor — *build big → capture → bake → render* — and
upgraded flowers from flat billboards to 3D shells. But #18 **explicitly deferred** the two
things that turn that capability into a *community* pipeline:

1. **The in-engine creator UI.** #18 bakes the *built-in* flowers via procedural Rust builders
   (`micro_model_assets.rs`); there is **no player-facing authoring loop**. A tester who thinks an
   asset looks wrong can do nothing about it.
2. **The do-ocracy / curation layer.** #18 deferred "how a community redesign becomes the game's
   art" as "a good problem to have, not now."

This doc designs both — but **narrowed by the owner's strategic frame** to the achievable,
multiplayer-safe 80%: **redesigning the appearance of assets that already exist.** It deliberately
does **not** build the heavy half (new block identities, animated creature rigging, in-game
voting) — those are parked behind this loop proving itself.

The IP/architecture reason this is the *right* first slice: a **reskin is a visual override**, not
a new game object. It needs no server agreement, no registry entry, no anti-cheat reasoning — it's
the resource-pack pattern. **New identities** crash into the hardcoded block registry + multiplayer
authority; those stay parked.

---

## Design

### The unifying model: the Workshop produces *overrides of existing assets*

Everything the Workshop makes in v1 is an **appearance override keyed to an existing asset type**:

| Verb | On a block | On a mob |
|------|-----------|----------|
| **Paper the faces** (Mode A) | block reskin | **mob reskin — same tool** |
| **Rebuild the shape** (Mode B) | block → 3D micro-model | (mob reshape = #19, **parked**) |

An override is **global** ("the whole flower everywhere"), **personal** by default (you see your
redesigns), **shareable** (open stash), and **graduatable** (owner bakes the best into the shipped
game). It is **render-only** — block ids, mob types, saves, inventory, crafting, behaviour are all
untouched.

### The bellows / pin authoring gesture

1. **Place** the asset at normal size — a block, a stack of blocks, or a **mob mannequin** (a
   still, posable instance of the mob).
2. **Bellows → inflate.** A pump tool blows the asset up to a comfortable working size — *like
   inflating a photo/bubble, never exploding it*. Each pump steps the scale; a valve lets it back
   down. (Owner picked the bellows over a shrink-ray / easel — playful + tactile + kid-legible.)
3. **Edit** — Mode A paints the (now-large, easily-clickable) faces; Mode B rebuilds the form.
4. **Pin → deflate + commit.** "Put a pin in it" pops the asset back to normal size **and commits
   the override** (to your local override registry — see §The persistent workspace). The pin is the
   explicit "I'm done with *this one*" moment. **You do NOT have to pin to leave** — an un-pinned
   project stays inflated and parked in the Workshop across sessions (next §).

For mobs, the mannequin carries a **"play" toggle** to preview its (already-built) walk/idle motion
between edits — the reskin rides the existing animation untouched.

### The Workshop is a persistent, multi-project workspace

The Workshop is **a saved world**, not a transient editor screen — so it inherits world persistence
for free. An inflated project is a **real, persistent object in the Workshop world**, not a
throwaway render overlay. This is the deciding design choice (it resolves the earlier "what *is*
blown up, in world terms?" question): **working copies are real, saved geometry + edit-state**,
which makes the "leave it and come back" workflow automatic.

Two distinct states — the difference is the whole point:

| State | What it is | Where it lives | Persists across sessions? |
|-------|-----------|----------------|---------------------------|
| **Parked WIP** | an un-pinned project — still inflated, half-edited | inside the **Workshop world** | **yes** — saved with the world |
| **Committed** | a pinned, finished redesign | the **override registry** (applies to your game) | yes — committed |

What this buys the author:
- **Leave and return.** Blow something up, work on it, walk away — it's still there, still inflated,
  next session. (It's just world state.)
- **Many in flight.** Keep as many inflated projects around the Workshop floor as you like — a cow
  here, three blocks there — each at its own stage. They're just objects in the world.
- **Finish on your own schedule.** Pinning is **per-project finishing**, not a "complete this now"
  gate. Pin one today, another next week.

Representation:
- **Mode B (reshape) WIP is just world geometry** — a blown-up project is a large-scale build of
  real blocks; saving the Workshop world saves it with zero extra work.
- **Mode A (reskin) WIP is paint-in-progress** — the partially-painted 16×16 faces are stored in a
  **`workshop_projects` side-table on the Workshop world** (serialised with the world,
  `#[serde(default)]`), so a half-painted cow survives a save/quit.
- A little per-project metadata (target asset, inflation level, Mode A vs B) rides alongside, so
  re-entering restores each project exactly where it was left.

### Resolution — 16, everywhere

- **Faces (Mode A):** **16×16** texels per face — a block's native texture resolution
  (`texture_gen.rs SIZE=16`). Blow the asset up so each face is a 16×16 grid of paintable cells.
- **Shape (Mode B):** **16³** sub-voxels (one micro-voxel ↔ one texel). 8³ exists only as a
  *lower-detail, cheaper* option — below native resolution, a deliberate downgrade, not the default.

### Mobs ride the block tool for free

A mob is a small stack of cuboid parts, each face pointing at a 16×16 block-texture-array layer —
*identical* to how a block face is textured (`entity_model.rs:212`, `:1549`). So **the Mode A
face-painter reskins mobs with no new texture system**: a mob is just a multi-part block. The only
mob-specific UX is *more faces to paint* (per part) and the mannequin/play preview. **Mob reskin
shares Phase A's override infra and Phase C's painter** — it is not a separate lift. (Crucially:
this **avoids** the player-avatar 64×64 atlas path, which would have been a renderer-architecture
change for zero benefit here.)

### The technical spine: a per-asset override registry

The one load-bearing new system. A **personal `OverrideRegistry`**:

- `block_tex_overrides: AHashMap<BlockId, AuthoredFaces>` — per-face authored 16×16 textures.
- `mob_tex_overrides: AHashMap<MobType, AHashMap<PartName, AuthoredFaces>>` — per-part faces.
- `block_shape_overrides: AHashMap<BlockId, MicroModelId>` — Mode B reshape (reuses
  `MicroModelRegistry` from #18 directly).

Applied at the existing seams:
- **Texture injection (Spec 03 §3.2):** after the procedural built-in textures are generated, the
  authored 16×16 override textures are **appended as extra layers** to the block texture array and
  an `(asset, face) → override_layer` map is built. (This is literally §3.2's "scan, build
  `name→layer`, upload array" + §3.3's "reference by name, resolve index at build time.")
- **Block faces:** the mesher consults `block_tex_overrides` **before** the default `tex_*`
  (`block.rs` `tex_side` etc.); applying/changing an override re-meshes loaded chunks (the existing
  dirty-chunk re-mesh path).
- **Mob faces:** the entity vertex builder consults `mob_tex_overrides` before `ModelPart.tex_faces`
  — read per-build, so it's automatic, no re-mesh.
- **Block shape:** reuses #18's existing `block_id → micro-model` override hook in
  `emit_non_solid_blocks` — Mode B just *populates* it from an authored micro-model.

The registry serialises (serde, versioned like `PlanData`) so it rides Stash; it carries
`author_npub` + a derivation chain for attribution (mirror #18 / the Plaque pattern).

### Distribution + the adoption pipeline (the actual "polish *the game*" payoff)

Three steps, **no heavy governance**:

1. **Personal** — your overrides apply to your own game immediately (local override set).
2. **Shared** — publish your override set to your **open stash** (reuse `open_stash.rs` kind-30820
   manifests + Blossom blobs; this is the first *asset* consumer of the worlds-only transport).
   "Pull from someone's open stash" = **adopt their redesign of an existing asset** (their cow
   becomes your cow).
3. **Official** — the **owner/team hand-pick** the best community overrides and **bake them into
   the shipped game art** (a curated "official overrides" bundle, `include_str!`-style like the
   built-in flowers + plans). This is the do-ocracy loop **without** in-game voting.

---

## Phased scope

Each phase is independently shippable behind `./check.sh`; the **look/feel** is the playtest
boundary. **Mode A (reskin) leads; Mode B (reshape) is the final phase.**

### Phase A — the override layer (the spine; no UI yet)
*Goal: an authored per-asset appearance override applies at render time. The resource-pack path,
scoped to overrides. Solo-verifiable, no Workshop UX yet.*

- New `override_registry.rs`: `OverrideRegistry`, `AuthoredFaces` (6× 16×16 RGBA), serde +
  `content_hash` (mirror `plan::content_hash`), `author_npub` + derivation chain.
- **Runtime texture injection** (Spec 03 §3.2): append authored 16×16 layers to the block texture
  array at load; build the `(asset, face) → layer` map.
- **Override lookup** at the two seams: block mesher (before default `tex_*`, re-mesh dirty chunks
  on change) and entity vertex builder (before `ModelPart.tex_faces`).

**Acceptance:** a test override re-textures one block id + one mob part at runtime (assert the
mesher/entity-builder emit the override layer); a block/mob with **no** override is byte-identical
to today; override set round-trips serde; `check.sh` green (clippy clean, native + WASM, all tests,
`trunk build`, bundle < 5 MiB).

### Phase B — the Workshop space + bellows/pin + persistent workspace
*Goal: enter a blank space, place an asset, inflate/edit/deflate — and **leave projects parked and
persisted across sessions, many at once.** No painting yet — the rig + the workspace.*

- A blank/void world preset (reuse world-gen with an empty preset) + enter/exit flow ("go to the
  Workshop" from the menu / a Workshop block). **The Workshop is a saved world** (reuse the existing
  world save/load), so it persists like any world.
- Place a block, a stack, or a **mob mannequin** (a still, non-AI mob instance).
- **Bellows** tool: stepwise inflate (+ valve to deflate); **pin** = deflate + (Phase C) commit.
  **Un-pinned projects stay inflated and parked** — leaving/saving the Workshop does NOT require
  pinning.
- **Project objects + persistence:** each inflated project is a real, saved object; a
  **`workshop_projects` side-table** on the Workshop world (serde, `#[serde(default)]`) records each
  project's target asset, inflation level, mode, and Mode-A paint-in-progress. **Many concurrent
  projects** supported.
- Mob mannequin **"play" toggle** (drives the existing walk/idle preview).

**Acceptance:** unit tests for the scale-step maths + state machine (placed → inflated(n) → parked
→ pinned); **a Workshop world with ≥2 parked, still-inflated projects round-trips through save/load
with each restored to its exact prior state** (offset, inflation, mode, partial paint); headless
screenshot of an inflated block (lobby-screenshot tooling, per `reference_lobby_screenshot_tool`);
`check.sh` green. **Playtest boundary:** the bellows/pin *feel* + the "potter across days" workflow
→ Axolittle.

### Phase C — Mode A face-painter → reskin (first real consumer)
*Goal: paint an inflated asset's faces at 16×16 and pin it into a global override. Blocks AND mobs.*

- A face-painting UX on the inflated asset: a 16×16 paint grid per face, the `WALLPAPER_*` palette
  ("paint-with-blocks"), per-cell colour. Pin → writes an `AuthoredFaces` into the override
  registry, keyed by `BlockId` (block) or `(MobType, PartName)` (mob).
- Wire pin/commit to Phase A's registry; apply immediately (personal/global).

**Acceptance:** painting a flower's faces and pinning makes **all** flowers render the new texture
(visual — owner's eyes); a repainted cow re-textures every cow; block id / mob type / saves /
behaviour unchanged (render-only); `check.sh` green. **Playtest boundary:** the painter UX + the
look → Axolittle. *This is the headline deliverable — community reskin works end-to-end, locally.*

### Phase D — Beacon: publish + discover + adopt override sets ✅ BUILT (2026-06-05)
*Goal: publish your overrides to the world and adopt others' — on `@forgesworn/beacon`.*

> **Migrated off `open_stash` onto Beacon** (`CONSUMING.md`): the prototype's Rust manifest logic
> (`OpenStashManifest`/`FollowedNpubs`/`build`+`parse_manifest_event`) + `openstash.js` +
> `official_content.rs` are **deleted**; the SDK owns build/parse/**schnorr-verify**.

- **Publish** (`/ws publish <name>` → `/ws publish confirm`): serialise the active `OverrideSet` to a
  version-prefixed bincode blob (`OverrideSet::to_blob_bytes`), stamp `author_npub` (hex, from the
  session pubkey), and publish a kind-30820 `d:axenstax` Beacon item (`contentType="override-set"`).
  Public-content gate is **two-step + explicit** (`beacon_publish_pending`, transient, never
  persisted — CONSUMING.md §7): a stage-and-warn, then an explicit `confirm`.
- **Discover + adopt** (`/ws follow|unfollow|following|browse|adopt`): NIP-51 follow set; browse lists
  `override-set` items across followed npubs (item cap); adopt fetches the blob →
  `apply_adopted_override_bytes` (version-check + texture-array layer guard) → `merge_adopted`
  (last-wins + one derivation link) → live re-texture + re-mesh.

**Acceptance (met):** `to_blob_bytes`/`from_blob_bytes` round-trip incl. provenance + reject a newer
version; `merge_adopted` last-wins + one link + stable appearance hash; `apply_adopted_override_bytes`
rejects garbage/newer/over-cap **without** mutating the registry; npub↔hex round-trips + the bech32
decoder rejects bad checksums; publish/browse/adopt are PWA-gated, native degrades; `check.sh` green.
**Owner step:** the live publish→follow→list→fetch→adopt round-trip from a second identity (no
relay/Blossom/signer on the build host). **Playtest:** the confirm-dialog + browse-screen feel.

### Phase E — adoption into official ✅ BUILT (2026-06-05)
*Goal: the owner-curated path from community override → shipped game art (offline, both platforms).*

- A bundled **official override catalogue** embedded via `include_str!`
  (`official_overrides.rs::official_catalogue`), loaded at world entry **beneath** personal overrides
  (`apply_official_beneath` — personal wins per key; official is a base layer, **no** derivation-chain
  bleed). Ships **empty** (`{"version":1,"items":[]}`) ⇒ byte-identical to a stock game until baked.
- **Build-time bake** (`tools/bake-beacon.js`, Node): reads the official npub's Beacon manifest +
  fetches each `override-set` blob (sha256-verified, bounded `MAX_ITEMS=64` / `MAX_BYTES=2 MiB`),
  emits the catalogue JSON (`bytes` as a u8 array — no base64 dep). No runtime Rust Beacon. Manual
  curation; **no in-game voting** — deferred.

**Acceptance (met):** the loader applies a fixture catalogue for a fresh player; a personal override
of the same asset **wins**; empty catalogue = byte-identical no-op; native + wasm both embed the data;
the bake tool bounds items + bytes and fails loudly on a sha256 mismatch; `check.sh` green. **Owner
step:** the live bake against the real official npub + wiring `window.__axenstax_official_pubkey_hex`.

### Phase F — Mode B reshape (rebuild as microblock)
*Goal: redesign an existing asset's **shape** as a global override, reusing #18's bake.*

- In the Workshop, build the new form (single-block, v1) → capture (`plan::capture`) → bake
  (`micro_model::from_plan` + `bake_micro_model`) → register as the asset's `block_shape_override`
  (populate #18's existing `MicroModelRegistry` override hook).
- **Multi-block sculptures (stacked cages, "touching = joined") are an Open Question / likely v2** —
  most existing assets are single-block or mobs, and `from_plan` is single-block today (see below).

**Acceptance:** rebuilding a flower's shape in the Workshop and pinning makes all flowers render the
new 3D micro-model (visual — owner's eyes); single-block exact-build enforced (multi-block refused
cleanly); `check.sh` green. **Playtest boundary:** the reshape look → Axolittle.

---

## Out of scope / deferred (explicit)

- **Brand-new blocks / items (new identities).** The block registry is hardcoded; runtime,
  data-driven block registration is a separate large spec. v1 redesigns **existing** assets only.
  *(The owner's "then we think about creating new items" — parked, "good problem to have".)*
- **Mob rebuild + reanimate (skeleton-attach).** That is **#19**
  (`2026-06-03-dynamic-asset-authoring.md`), blocked behind #18's playtest. **Mobs in v1 = reskin
  only.** Skeleton-attach **parked** (owner, 2026-06-04).
- **New placeable items + inventory-placement flow.** Dissolved for v1 — every override is bound to
  an existing asset, so nothing new needs an inventory home.
- **Multi-block micro-model sculptures.** `from_plan` is single-block exact-build; multi-block needs
  a new data type + bake + render. Open question; likely v2.
- **In-game community voting / "asset-show" competition governance.** Adoption is owner-curated for
  v1. The provenance plumbing (author npub + derivation chain) ships now so governance is a drop-in.
- **Behaviour/hitbox changes.** Overrides are **visual only**; collision/AI/stats unchanged →
  multiplayer-safe, server stays authoritative.
- **The Workbench rename** (crafting table → Workbench) — adjacent naming change, Spec 05 surface,
  separate.

---

## Risk / confidence

**Overall: L–XL effort, Medium-High confidence on the reskin loop, Medium on authoring UX.**

- **The reskin path is well-grounded.** Mobs-are-textured-like-blocks is the key de-risker: one
  painter, one override layer, both creatures — no avatar-atlas rewrite. The override layer *is* the
  already-designed resource-pack path (Spec 03 §3.2–3.3).
- **The one genuinely-new render risk** is runtime texture-array injection (appending authored
  layers + re-meshing on apply). Bounded, and Spec 03 already specifies the shape; the block
  re-mesh reuses the existing dirty-chunk path.
- **Authoring UX is the soft spot** (as in #18/#19): the face-painter + bellows/pin *feel* can't be
  solo-verified → Axolittle playtests gate Phases B/C/F.
- **Stash is "built but unproven"** (`project_cloud_save_bridge_on_main`): the transport exists but
  has never run end-to-end and only knows worlds. Phase D adds the *asset* shape; the live round-trip
  is an owner step (no Docker on host).
- **WIP persistence is low-risk** — the Workshop is a world, so parked projects ride the existing
  world save/load (append-only by invariant, `project_goal3_save_hardening_delivered`). The only new
  serialised state is the `workshop_projects` side-table (Mode-A paint-in-progress + per-project
  metadata), `#[serde(default)]` so old/empty Workshops load clean.
- **Multiplayer-safe by construction** — visual-only overrides need no authority/anti-cheat
  reasoning; that's why this slice is the right first one.
- **WASM:** painter + override apply are CPU + the existing texture-array/mesh paths (already on
  WASM). No native-only deps. Same cross-platform posture as #18 / cosmetics.

---

## Open questions (resolve during build, not blocking)

1. **Multi-block reshape in v1 or v2?** Recommend **single-block only for v1** (matches `from_plan`),
   defer stacked-cage sculptures — most *existing* assets are single-block or mobs.
2. **Override conflict resolution on adoption.** Last-adopted-wins vs per-asset pick vs a layered
   "active pack" model. Start simple (last-wins + a per-asset on/off), refine on feedback.
3. **Texture-array growth.** Appending authored layers grows the array — cap per player + reuse
   layers on override-replace (don't leak layers); honour the same array-limit care the cosmetics
   native-crash note flagged (`project_player_cosmetics_plan`).
4. **Where the bellows/pin lives** — a held tool vs a Workshop-only mode interaction. Tool is more
   discoverable; decide with Axolittle.
5. **Mannequin source** — spawn a non-AI mob instance vs a dedicated "display" entity. Non-AI
   instance is cheapest; confirm it animates for the "play" preview without AI ticking.
6. **One Workshop or many?** v1: a single per-player Workshop world holding **many parked projects**
   (covers "lots of things on the go"). Multiple *named* Workshops (themed benches) is a trivial
   extension if wanted later — it's just more saved worlds.
7. **Mode-A WIP granularity** — store paint-in-progress as full 16×16×6 face buffers per project
   (simple, a few KB each) vs a diff against the asset's current texture. Full buffers for v1;
   optimise only if a Workshop accrues many parked paint projects.

---

## File-touch map (when built — NOT now)

| File | Phase | What |
|------|------|------|
| `game/engine/src/override_registry.rs` (new) | A | `OverrideRegistry`, `AuthoredFaces`, serde, `content_hash`, provenance + tests |
| `game/engine/src/texture_gen.rs` / texture-array upload | A | append authored 16×16 override layers; `(asset,face)→layer` map (Spec 03 §3.2) |
| `game/engine/src/mesh.rs` | A | block-face override lookup before default `tex_*`; re-mesh-on-apply |
| `game/engine/src/entity_model.rs` | A | mob-face override lookup before `ModelPart.tex_faces` |
| Workshop space + bellows/pin (new module + menu/world-preset hooks) | B | blank preset, enter/exit, place asset/mannequin, inflate/deflate state machine, play toggle, **`workshop_projects` parked-WIP model** |
| `game/engine/src/save.rs` / world save | B | persist the Workshop world + `workshop_projects` (parked, still-inflated WIP) — `#[serde(default)]`, append-only invariant |
| face-painter UX (new module + `plan_ui.rs`/menu hooks) | C | 16×16 per-face paint grid, `WALLPAPER_*` palette, pin → write override |
| `game/engine/src/open_stash.rs` / `openstash.js` / `cloud.js` | D | `kind:"override-set"` blob + manifest; publish + adopt |
| official-overrides bundle (`assets/overrides/*` + loader) | E | `include_str!` built-in override set, applied beneath personal |
| `game/engine/src/micro_model*.rs` (reuse) | F | Mode B: capture→bake→register as `block_shape_override` (single-block v1) |
| `docs/spec/03-rendering.md` | A–F | record the override layer as §3.2–3.3's first concrete consumer |
| `docs/spec/05-gameplay-systems.md` | B,C | the Workshop space + authoring loop |

*(All new types additive + `#[serde(default)]` where they touch saved structs — the `PlanData`
forward-compat pattern. Reuses #18's bake + capture; does not re-derive either.)*

---

## Memory-rule check

- **`project_shared_infra_strategy` (cross-game lift):** the override registry, runtime
  texture-override path, the Workshop space, and the adoption pipeline are **engine-generic** — any
  Decented voxel game gets community visual-polish from the same primitive. The *assets* are
  AxeNStax's; the *pipeline* is shared infra. ✅
- **`feedback_autonomy_to_playtest_boundary`:** Phase A + the registry/Stash logic are
  solo-buildable + `check.sh`-gated; Phases B/C/F end at a **look/feel** confirmation
  (Axolittle/owner) — build to the boundary, stop cleanly. ✅
- **`feedback_merge_to_main_preauthorised`:** healthy-gate merges to main pre-authorised, scoped to
  decented/axenstax. Design-only now — nothing to merge. ✅
- **`project_economies_vision` / Knowledge & Creator economy:** community-authored visual content is
  the Knowledge-economy lane; creator-sold *overrides* (Bitcoin) are a deferred payoff, not designed
  here (and settlement stays `project_settlement_model_decision_parked`). ✅
- **`reference_proof_of_play_is_proof_of_work`:** any future "redesign show" prize is
  skill/judging-based, not chance-based — consistent with the no-gambling rule. Deferred, untouched. ✅
- **`project_cloud_save_bridge_on_main` / `project_stash_locket_save_architecture`:** Phase D is the
  first *asset* consumer of the worlds-only Stash transport; the live round-trip is an owner step
  (no Docker on host) — flagged honestly, not assumed working. ✅
- **`feedback_npub_only_display`:** author attribution renders npub (NIP-19), never hex. ✅
- **`feedback_uk_english_naming`:** UK English throughout (colour, not color). ✅
- **IP (per the 2026-06-04 naming chat):** "The Workshop" is a generic, IP-clear name (the
  "Construct" option was dropped for Matrix + Scirra-engine trademark proximity); reskins are
  our-art / community-art only; mob reskins reuse block texturing, not the avatar atlas. ✅
- **No build yet:** design/spec only, per the project rule "do NOT build anything unless explicitly
  asked." The owner says when to action it. ✅
