# Texture-pack authoring — spec + architecture (DECIDED, ready to build)

**Status:** DECIDED — owner delegated the architecture call 2026-06-18 ("as
always, build for the long term"; resolution + WASM both for the long term).
Ready to build; phasing in §5.
**Date:** 2026-06-18. **Driver:** owner — "we really need to be able to create
new texture packs, because there are many textures which are [bad]."
**Source of truth:** this doc + Spec 03 §§2, 3, 11 (update §11 as phases land).

> This was the one item from the 2026-06-18 Workshop fix batch that was **not
> built**, because it's a genuinely large feature and shipping it half-baked
> would violate *Code Quality: Concrete, Not Cards*. The other four items in
> that batch (Bellows obtainable, Paint/Sculpt mode, paint lockdown,
> hide-original-on-blow-up) shipped live. See §7 for the interim path.

## TL;DR

There is **no texture-pack system today** — every one of the ~399 block
textures is generated procedurally in Rust (`texture_gen.rs`) and uploaded to a
single GPU texture array at startup. Spec 03 §11 fully specifies a real
resource-pack loader; **none of it is built.** The decided build is **not
greenfield**: a texture pack is *one more source in the override resolver that
already ships* (`official_overrides::resolve_render_set`), and the atlas builder
implements Spec 03 §3.2 as already written (resolve-per-name → upload), of which
the Workshop's append-above-base layer is explicitly the "first concrete
consumer" (Spec 03 §3.2 note). Resolution agility lives entirely at the
**pack/atlas layer** (Spec 03 §2/§3/§11) and is **fully decoupled** from the
persisted `OverrideSet` blob — so there is **no blob migration, now or ever**,
for this feature.

## 1. Current state (verified in code, 2026-06-18)

- **Generation:** `texture_gen::generate_textures()` returns `Vec<Vec<u8>>` of
  399 RGBA 16×16 layers, all procedural; no disk PNGs, no image decoding. Cached
  in a `OnceLock`. `SIZE: u32 = 16` is hard-wired (`PIXELS = 16*16*4`).
- **GPU bind:** uploaded as one 2D texture array at init (`renderer.rs`). A
  `BlockDef` maps a block to three layer indices: `tex_top`/`tex_bottom`/
  `tex_side` (`block.rs`) — i.e. **textures are keyed by layer index, not name.**
- **Resolver that already exists:** `official_overrides::resolve_render_set` is
  a **load-order resolver** composing three layers today — embedded official
  catalogue + per-player wardrobe + per-world override — into one
  `OverrideRegistry`. This is the embryonic "pack stack."
- **Authored-data format:** `OverrideSet` (`override_registry.rs`) is the
  serialisable, provenance-bearing (`author_npub` + derivation chain),
  bincode-versioned (v1→v2, migration concentrated in `from_blob_bytes`) blob.
  It is the wire format for Beacon publish/adopt, the embedded official
  catalogue, AND the persisted wardrobe (Stash on WASM / `profile/` on native,
  via `wardrobe_store`). `world.dat` carries no `OverrideSet` — appearance is
  **render-only**, which is what makes the whole path multiplayer-safe.
- **Atlas append (the bridge):** authored 16×16 reskins are **appended as extra
  layers** above the 399 base layers (`Renderer::rebuild_block_textures`,
  upload at `texture_count() + i`), with an `(asset, face) → layer` map consulted
  at the two render seams (`mesh.rs` greedy pass + `entity_model.rs`). Spec 03
  §3.2 names this the "first concrete consumer" of the atlas-rebuild mechanism.
  Hard-wired `FACE_BYTES = 16*16*4`; painter grid is 16×16.
- **`image` crate:** `image = "0.25"` is already a dependency (not native-gated),
  so in-WASM PNG decode adds no new bundle weight.

## 2. The gap (spec vs built)

Spec 03 §11 wants: pack directories (`pack.json` + named PNGs), runtime
hot-swap, pack layering (user > server-suggested > built-in), download+cache of
a server-suggested pack, resolution agility (16/32/64/128, scale-on-load),
animated textures (vertical strips + `.anim`), plugin texture integration.
**Built: none of it.** Built instead: the per-asset override append layer (§1).

## 3. Decided architecture

The unifying decision: **a texture pack is one more source feeding the existing
`resolve_render_set` resolver, and the atlas is built by resolving every texture
*by name* and uploading the result — Spec 03 §3.2 as written.** No second
pipeline beside the Workshop path; the Workshop append becomes a degenerate case
of the general atlas build.

### 3.1 Texture-key namespace (the foundation)

Textures become **name-addressed** (Spec 03 §2: "referenced by name, never by
pixel coordinate"). Introduce a stable string key per texture
(`blocks/stone`, `blocks/oak_planks_side`, `entity/cow`, …) and a registry
mapping key → source. `BlockDef.tex_*` and `entity_model` resolve through the
registry instead of holding raw layer indices. Keys are stable across
block-registry waves (they survive a rebuild; layer indices do not). The
`--dump-textures` filenames are exactly these keys.

### 3.2 The resolver — packs as sources

`resolve_render_set`'s load order generalises from 3 fixed layers to an ordered
list of **sources**, each answering "do you supply texture *key*?":

```
Priority (highest first):
  1. Personal Workshop wardrobe   (OverrideSet, 16×16, the player's own authoring)
  2. Server-REQUIRED pack         (§11.6 — non-negotiable on that server)
  3. User-selected pack(s)        (PNG, pack resolution, user preference order)
  4. Server-SUGGESTED pack        (§11.6, if accepted)
  5. Embedded official catalogue  (OverrideSet, 16×16)
  6. Procedural base              (generated, 16×16)
```

(Priority of personal wardrobe vs a *required* server pack is the one tunable
here; default above puts the player's own work on top, a required pack still
wins over suggested/user packs. Revisit if a themed server needs to force its
look over personal reskins.) For each texture key, the highest-priority source
that supplies it wins; otherwise fall through to procedural. A pack only ships
the textures it overrides (§11.4).

Pack sources and `OverrideSet` sources coexist in the same resolver: an
`OverrideSet` supplies 16×16 authored faces; a pack supplies named PNGs at the
pack's resolution. Both are resolved into final per-key images.

### 3.3 Resolve-then-upload atlas (retire append-above-base)

The atlas builder implements Spec 03 §3.2 directly: resolve every key to a final
image, scale to the active resolution, build `key → layer_index`, upload the
array, generate mipmaps, mark all chunk meshes dirty + reload entity/UI
textures. **This replaces generate-then-append.** The append model works only
because overrides are sparse (a wardrobe has a handful of active reskins); a full
~399-key pack appended above 399 base layers would blow the texture-array layer
cap. Resolve-then-upload bounds the array to the number of distinct keys
regardless of how many sources contributed. The Workshop's per-asset reskin is
then just "this key resolved to an authored image" — one pipeline, not two.
Touches: `renderer.rs` (array build), `mesh.rs` + `entity_model.rs` (layer
lookup keyed by name→index), `override_registry.rs` (feed the resolver).

### 3.3a Post-P2a finding — resolve-then-upload is NOT needed (P2b re-scoped)

**The §3.3 rationale above was written pre-build and is now superseded.** P2a's
`apply_pack_overrides` **replaces base layers in place** (`layers[i] = new_pixels`),
it does not append. So a full 400-key pack adds **zero** new array layers — the
texture-array layer cap is never at risk from a pack, which was the whole
motivation for resolve-then-upload. The append-above-base path remains in use
**only** for the Workshop's *sparse per-(asset,face)* overrides, where it is the
correct mechanism: divergent faces of one block genuinely need distinct layers,
which per-key replacement cannot express.

The shipped architecture (P1 + P2a) is therefore already clean — a **2-stage
layered composition**, not two drifting pipelines:
1. **Base resolution** — `base_textures()` resolves every layer by key: procedural
   ⊕ active disk pack (in place). One source, pack-aware, feeds **both** the
   initial upload and the Workshop rebuild.
2. **Sparse fine overrides** — the Workshop `OverrideSet` (via
   `resolve_render_set` + append) layers per-(asset,face) reskins **on top of**
   the pack-resolved base.

**Recommendation: drop P2b.** Folding the Workshop append into a single
resolve-by-key pass would be high-risk churn against live, deployed, persisted
Workshop data for no functional gain. The only residual bridge is P1's
`TEXTURE_KEYS` parallel list (vs the generator's push order); fold it into one
keyed table opportunistically when P3 next touches the atlas build — low value,
low risk, not blocking.

### 3.4 Resolution agility — pack/atlas layer only, NO blob migration

Resolution is a property of the **active pack + atlas**, never of the authored
per-face type (Spec 03 §2/§3/§11):

- `pack.json` declares `texture_resolution` (16/32/64/128, square, power-of-two).
- The texture array is allocated at the **active pack's** resolution; swapping
  packs reallocates (Spec 03 §3.2, §11.5).
- Sources at a different resolution are **scaled at load** — nearest-neighbour
  upscale to preserve pixel art, bilinear downscale (§11.3). So the procedural
  base (16×16) and the Workshop wardrobe (16×16) upscale cleanly into a 32/64/128
  atlas; no source needs to natively match the atlas resolution.

Consequence: **the `OverrideSet` blob stays 16×16 and is never migrated for this
feature.** The painter also stays 16×16 (a hi-res *painter* is a separate,
out-of-§11 feature; if it ever ships, *that* is when a future blob migration
happens — with its real requirements known, and against blobs that migrate
lazily per-read so corpus size is irrelevant). This is the long-term-correct
resolution build with zero migration risk.

### 3.5 Animated textures

Pack/atlas-level, orthogonal to the blob: a vertical strip
(`resolution` wide × `resolution · frame_count` tall) + a `.anim` sidecar
(§11.3). The name-addressed atlas leaves room; deferred to P5.

## 4. WASM asset story — DECIDED: fetch + decode (Spec 03 §11.6/§11.7)

Native reads a pack dir from disk trivially; the PWA/WASM build has no
filesystem, so packs are **fetched over HTTPS and decoded in-WASM**:

- Fetch from the game site (`/static/packs/<name>/…`) or a server-suggested URL;
  decode with the `image` crate (already in the bundle — no new weight).
- Cache by **SHA-256 hash**, not name, so versions coexist (§11.7);
  Cache API / IndexedDB on web, `resource_pack_cache/` on native.
- Server-suggested packs arrive via a `ResourcePackSuggest` packet (URL + hash +
  size + required flag), §11.6; integrity verified against the hash.
- Composes with Blossom/Stash for *user-authored* packs later (a pack is just a
  set of named blobs + a manifest).

Rejected: (4a) embed the full default pack — duplicates procedural generation
more compactly, pointless. (4c) in-engine-only, no files — too weak an
"import a PNG pack from outside" story for the owner's stated need.

## 5. Phasing (long-term build, end to end)

- **P1 — Texture-key registry. ✅ DELIVERED 2026-06-18** (`texture_registry.rs`,
  check.sh green / 2966 tests). Name-addresses all **400** layers via a name↔index
  authority — `texture_keys()` / `texture_index(key)` / `texture_key(index)` —
  keyed by the Spec 03 §11 taxonomy (`blocks/…`, `entity/<mob>/…`, `item/…`,
  `overlay/…`, `decor/<family>/<colour>`, `_retired/…`). The generator is
  **untouched**: consumers keep integer layer indices (the correct runtime
  address; the registry maps names → those indices, which is what packs/dump
  need). Dev-only `--dump-textures [out_dir]` writes one PNG per key (verified:
  400 PNGs, valid 16×16 RGBA, nested dirs) — the canonical default pack P2 reads
  back. The key list is a marked `// BRIDGE` (parallel to the generator's push
  order, bound by tests) to be folded into one keyed table in P2.
- **P2a — Native disk pack loader (THE owner's "fix bad textures" loop). ✅
  DELIVERED 2026-06-18** (`texture_registry.rs`, check.sh green). At texture-array
  build, a configured pack dir's `<key>.png` files override the matching base
  layers by name (`apply_pack_overrides`); missing/malformed/wrong-size files
  skip to procedural. Routed through one canonical `base_textures()` so **both**
  the initial upload (`main.rs`) and the Workshop rebuild (`renderer.rs`) reflect
  the pack. Pack is selected via `AXENSTAX_TEXTURE_PACK=<dir>` (native dev hook;
  P3 replaces it with a picker UI). **Reload = restart** (env re-read); live
  hot-swap is P3 (§11.5). So the loop is now closed: `--dump-textures` → edit a
  PNG → restart → the texture changes, no recompile. Tested: override-by-name,
  skip bad/missing, `base_textures_from` composition. *Not routed yet (P2b/P3):
  the `--shot-*` dev tools + Workshop painter preview still show procedural base.*
- **P2b — Resolve-then-upload atlas + OverrideSet fold. ❌ RE-SCOPED / DROPPED**
  (see §3.3a). The layer-cap rationale is moot — P2a replaces base layers in
  place (0 new layers per pack), and the shipped P1+P2a design is already a clean
  2-stage layered composition. Folding the Workshop append into one resolver
  would be high-risk churn against live deployed Workshop data for no functional
  gain. Skip it. (Residual: P1's `TEXTURE_KEYS` bridge → fold opportunistically
  in P3.) **The genuine next step is P3.**
- **P3a — `pack.json` manifest. ✅ DELIVERED 2026-06-18** (`texture_registry.rs`,
  check.sh green / 2973 tests). `PackManifest` (Spec 03 §11.2: name, description,
  version, `texture_resolution`, authors, license) with serde defaults;
  `load_manifest()` reads `<dir>/pack.json` or returns a sane default (16×16,
  name from the dir); `texture_resolution` outside {16,32,64,128} clamps to 16
  (never bricks load). `--dump-textures` now also writes a default `pack.json`, so
  a dumped pack is a self-describing named unit. Resolution is parsed but not yet
  consumed — that's P3b.
- **P3b — resolution-agnostic atlas + scale-on-load. ✅ DELIVERED 2026-06-18**
  (check.sh green / 2975 tests). `scale_layer` (nearest-up to preserve pixel art,
  bilinear-down, identity when equal) per §3.4/§11.3; `base_textures_from` reads
  the manifest resolution and upscales the procedural base to it;
  `apply_pack_overrides` scales any *square* pack PNG to the atlas resolution
  (non-square skipped); the renderer infers `tex_size` from the layer bytes
  (`square_side`) — no signature change — and scales the Workshop's 16×16 appended
  layers to match so the array stays uniform. Packs can now ship at 32/64/128.
  **Strong safety property: with no pack (or a 16×16 pack) every scale is identity,
  so the default game is byte-for-byte unchanged.** Hi-res visual crispness = owner
  boundary (set `texture_resolution` in pack.json + paint at that size).
- **P3c — pack picker UI + P3d hot-swap. ✅ DELIVERED 2026-06-18** (check.sh green
  / 2977 tests). Packs are discovered from a `texturepacks/` folder (alongside
  `worlds/`); a "Texture pack" dropdown in the in-game Graphics settings panel
  lists Default + installed packs. Selecting one activates it
  (`select_pack` → `set_active_pack` + persisted to `texturepacks/active.txt`)
  and the panel returns `TexturePackChanged`, which calls the existing
  `rebuild_overrides_and_remesh` — **live atlas rebuild + remesh, no restart**
  (§11.5). Startup re-activates the saved pack before the first atlas build.
  `AXENSTAX_TEXTURE_PACK` stays as a dev override (highest precedence). Feel/visual
  = owner boundary. *(Full multi-pack load-order layering §3.2 deferred — single
  active pack for v1.)*
- **P4 — WASM fetch + decode + cache (§4) + server-suggested packs (§11.6).**
  Split into shipped + remaining:
  - **P4a — cross-platform pack-application core. ✅ DELIVERED 2026-06-18.**
    `DecodedTexture` + `apply_pack_layers` (`texture_registry.rs`): the
    override-by-name compositing extracted out of disk I/O so native + WASM share
    one path. `decode_pack_dir` = the native half.
  - **P4b — SHA-256 cache primitives + `ResourcePackSuggest` packet. ✅ DELIVERED
    2026-06-18.** `resource_pack.rs`: `pack_hash` (lowercase-hex sha-256, the
    cache key + integrity value), `verify_integrity`, `lru_evictions` (500 MB
    budget, §11.7) — all pure + unit-tested. `protocol`: `ResourcePackSuggest`
    packet (tag 52) + PROTOCOL_VERSION 52→53 + round-trip test.
  - **P4d — WASM web pack swapping. ✅ DELIVERED 2026-06-18** (check.sh green incl.
    `trunk build`). The web build can now swap packs: a web "Texture pack" picker
    in Graphics settings lists packs from the game site's `/static/packs/index.json`;
    selecting one fetches its `<key>.png` files (`texturepacks.js` JS bridge),
    decodes them in-WASM (`image` crate), and installs them via
    `texture_registry::set_wasm_pack` → the existing atlas-rebuild path applies
    them (`texture_packs_web.rs`). Selection persists in localStorage and
    re-applies on reload. A bundled sample pack (`tools/texturepack-sample/` →
    `static/packs/vivid/`) makes it exercisable. *Live browser fetch + visual =
    playtest boundary.* Web **animated** textures + a bespoke hash-keyed byte
    cache are follow-ups (web takes an animated strip's first frame for now; the
    browser HTTP-caches same-origin packs, so §11.7's hash cache matters mainly
    for P4c's arbitrary-URL server packs).
  - **P4c — server-suggested packs (§11.6). ✅ SPINE DELIVERED 2026-06-18**
    (check.sh green / 3004 tests). The §11.6 wire contract + policy are live and
    tested: the server sends a configured `ResourcePackSuggest` once the join
    handshake completes (`hosted_server`, native; operator config via
    `AXENSTAX_PACK_URL` + `_NAME`/`_SHA256`/`_SIZE`/`_REQUIRED`, built by
    `resource_pack::configured_suggestion`); the client receives + stores it
    (`RemoteClient.pending_resource_pack`) and the game loop surfaces it as a chat
    line. The pure decision policy `resolve_suggestion_outcome(required, accepted)`
    (Apply / KeepCurrent / Disconnect) + `build_suggestion` are unit-tested.
    **DEFERRED (infra + playtest gated):** the interactive Accept/Decline prompt
    and the fetch-on-accept of an arbitrary-URL pack — that needs a hosted pack
    artifact + a native HTTP client (none today; native saves are local) + the
    hash-keyed byte cache (`verify_integrity`/`lru_evictions` are the ready
    primitives). On accept, the web path can reuse the P4d in-WASM fetch/decode;
    native download is the remaining transport. A live 2-machine accept is the
    playtest boundary.
- **P5 — Animated textures (§3.4). ✅ DELIVERED 2026-06-18** (`texture_anim.rs` +
  `texture_registry::collect_pack_animations` + renderer
  `advance_animated_textures`, check.sh green). Vertical-strip `<key>.png` +
  `<key>.png.anim` sidecar → per-layer frame upload driven by the world
  `tick_counter` (game time, pauses with the game), capped at 32. Frame 0 seeds
  the static base layer so a pack without the per-frame loop still looks right.
  Pure logic fully unit-tested; visual smoothness = playtest boundary.
  **Plugin texture integration (§11.8) is DEFERRED — genuinely blocked:** there is
  no runtime block-registration / plugin system to integrate textures with
  (`BlockRegistry` is compile-time static, verified 2026-06-18). It is a card
  until a plugin system exists; revisit when one ships.

P1+P2 deliver the owner's immediate need (author/replace bad textures from
outside the engine) as the **foundation** of the full system, not a throwaway.
P3+ complete Spec 03 §11.

## 6. Cross-game / shared-infra note

A name-addressed texture registry + resolver-fed pack loader is engine-generic;
keep it free of AxeNStax-specific assumptions (`project_shared_infra_strategy`).
The `OverrideSet`/Beacon provenance model already generalises — packs inherit it.

## 7. Interim path (available now)

The **Workshop reskin is the texture editor that exists today**, and the
2026-06-18 batch removed the bugs that blocked it: the Bellows is obtainable
from the creative inventory, painting is a clean dye-only single-pixel operation
in Paint mode, carving only happens in Sculpt mode, and the blown-up block no
longer shows its original texture through the working copy. So a bad texture
*can* be repainted per-block right now — this spec turns that into authorable,
swappable, importable **packs**.

## Memory-rule check

- Spec source of truth: this doc + Spec 03 §§2/3/11 (update §11 as phases land).
- Shared-infra: pack loader is cross-game; engine-generic (§6).
- Confirm-before-CI: building this needs no CI; `--dump-textures` is a local tool.
- No blob migration: decided architecture keeps `OverrideSet` 16×16 (§3.4) — do
  NOT add a resolution field to the persisted authored-data type for this feature.
