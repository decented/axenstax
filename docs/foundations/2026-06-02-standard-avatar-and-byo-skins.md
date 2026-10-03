# Player Cosmetics — standard avatar now, player-owned cross-game cosmetics as the destination

**Status:** ✅ BUILT — Phases 1–4 merged to main (`cosmetics.rs` / `skin_uv.rs`); standard avatar + upload-your-skin (WASM) live. Networked 2-client demo = platform-gated. Reconciled 2026-06-19. Owner-requested after the cloud-save ("Stash") work; reframed after community/market research (see §Strategic opportunity) showed the real prize isn't "a better skin uploader" — it's **player-owned, creator-monetisable, cross-game cosmetics**, which Mojang/Microsoft structurally cannot ship. Round one is still the simple 64×64 skin; the architecture is shaped so the bigger play isn't foreclosed.

**Trigger:** Three layered goals — (1) **now:** make the player avatar use the **standard 64×64 humanoid skin layout** (interop *format*, our own art) so the whole skin-editor ecosystem works; (2) **now:** let players **upload their own skin**, stored per-persona in Stash so it follows them to any device; (3) **future, unblocked-not-built:** richer cosmetics (3D models, overlays, capes, emotes) that are **owned by the player's Nostr persona** and (with Bitcoin) **sold directly by creators with no marketplace cut** — across our games. The build below makes (1)+(2) ship while keeping (3) a drop-in, not a rewrite.

**Branches:** fork off `main` → `feat/standard-avatar` (Phase 1), then `feat/byo-skins` (Phase 2). Phase 2 depends on Phase 1 + the Stash cloud-save work. Phase 3+ (richer cosmetics) are separate later specs that build on the same descriptor.

---

## TL;DR

The avatar is **already 6 cuboids** — structurally a Minecraft humanoid. Two things make it non-standard: proportions are off the 8/8/8 (head), 8/12/4 (body), 4/12/4 (limb) ratios, and texturing uses **6 separate procedural 16×16 layers** with full-face UVs instead of one **64×64 atlas** with the standard box-unwrap.

Round one: nudge proportions, replace the 6-layer texturing with a **64×64 box-UV unwrap** (+ the standard overlay/hat layer), then BYO-skins = "upload a 64×64 PNG → it's your skin → Stash carries it to any device."

**But build it data-driven.** The avatar's appearance is read from a per-persona **cosmetic descriptor** (a Stash asset), not hardcoded. Round one the descriptor is just `{ skin: <64×64 png> }`. Later it grows — `{ model, skin, overlays, cape, emotes }` — **without re-architecting** renderer or transport. And **the visual model is decoupled from the hitbox from day one**, so cosmetic geometry can never affect server fairness (the exact reason Mojang froze custom models — designed out here). See §Design foundations.

**Two crux decisions, both below:**
- **Phase 0:** our texture system is a hardcoded 16×16 D2 array; a skin is a 64×64 atlas with sub-rect UVs → the avatar moves to a dedicated 64×64 texture path (Option B).
- **Cosmetic descriptor + hitbox/visual separation** (§Design foundations) → the future-proofing that makes Phase 3+ a drop-in.

**IP posture:** the 64×64 UV layout is a *functional interop spec* (like reading .docx) — adopting it is fine. Never ship Minecraft's default skins (Steve/Alex) or bundle/scrape any skin library. Ship **our own original** default; users upload the rest. The moment cosmetics become *sellable*, plan for user-generated-content moderation. See §IP.

---

## Strategic opportunity (research, 2026-06-02)

We researched what the Minecraft community wants and what Mojang/Microsoft won't do, to design toward where the puck is going rather than matching today's standard.

**What the community demonstrably wants** (download counts = proof of demand):
- **Custom 3D player models, not flat skins** — the #1 ask. [Customizable Player Models](https://www.curseforge.com/minecraft/mc-mods/custom-player-models) ~5.4M downloads; [Figura](https://modrinth.com/mod/figura) (3D + animation + emotes + scripting, syncs P2P without server mods) ~1.4M.
- **Cosmetic geometry that doesn't change the hitbox** — an explicit [Minecraft Feedback request](https://feedback.minecraft.net/hc/en-us/community/posts/39727949481869-Allow-Custom-Skin-Geometry-that-Does-Not-Affect-Hitboxes): hats/capes/ears as visual-only.
- **Emotes** ([Emotecraft](https://www.curseforge.com/minecraft/mc-mods/emotecraft-forge)) and **custom capes** ([CapeMod](https://modrinth.com/mod/capemod), Lunar) — all community-built because vanilla refuses.
- **Cosmetics that follow you across servers** — only exist via third-party launchers (Lunar) that silo them.

**What Mojang/Microsoft won't do — and why (this is the gap):**
- **Anti-cheat fear.** Custom models were added to Bedrock then *removed* over "could be used to cheat." They froze the creative space to avoid a server-fairness problem.
- **Revenue protection.** The Bedrock [Marketplace](https://www.minecraft.net/en-us/partner) has paid creators **$500M+** with Mojang taking a cut, walled to Bedrock and partner-gated. **Java creators can't sell cosmetics at all.** Richer free/open cosmetics would cannibalise the store.
- **No identity/payment rails.** No per-player decentralised identity, no micropayments → "you own this cosmetic and can carry/sell it anywhere" is structurally impossible for them.

**The tell:** CPM smuggles custom models into *unused pixels of the skin PNG or a GitHub Gist*; Figura runs its *own backend* to sync avatars — the community is jury-rigging a transport that doesn't exist.

**Why this is uniquely ours.** Every blocker maps onto rails we already have or are building:

| Community pain | Mojang can't / won't | We have |
|---|---|---|
| Models smuggled in skin pixels / Gists | no avatar-data transport | **Stash** — per-persona encrypted asset store that follows your npub to any device |
| Creators can't sell (Java) / walled-marketplace cut (Bedrock) | protects the $500M garden | **Bitcoin/Lightning + Nostr** — creator sells direct, no cut, not a money transmitter |
| Cosmetics siloed per-launcher | no decentralised identity | **Nostr persona** — cosmetics owned by *you*, interoperable across our games |
| Custom geometry blocked over cheating | server-fairness fear | **server-authoritative engine** — visual/hitbox separation by design |

So the "do it better" is **player-owned, creator-monetisable, cross-game cosmetics** — the thing Mojang structurally cannot ship. Round one doesn't build that; it makes sure we don't foreclose it.

**Honest caveats for the destination:** (1) richer 3D models reintroduce a **performance** budget (Figura avatars can be heavy) — our plant-instancing work helps but it's a real constraint to design for; (2) once cosmetics are *sellable*, **UGC IP exposure** grows (people will try to sell infringing models) — a moderation/terms problem to plan for, not a blocker.

## Design foundations (the future-proofing — applies from Phase 1)

Two principles that cost little now and unlock everything later. **Build round one to honour these even though round one only needs a flat skin.**

### F1. Avatar appearance is a data-driven **cosmetic descriptor**, not a hardcoded model
The renderer builds the avatar from a per-persona descriptor object, stored as a Stash asset. Round one:
```
CosmeticDescriptor { version: 1, skin: <64×64 png ref> }
```
Later (no re-architecture — additive fields, version-bumped):
```
CosmeticDescriptor { version: N, model?: <geometry>, skin, overlays?, cape?, emotes?, ... }
```
Implication for Phase 1: introduce the descriptor type + a `default` value now, and have the avatar render read *from it* (even though it only carries `skin`). Don't hardcode "the skin is texture layer X"; route through the descriptor. This is a small indirection that turns Phase 3 from "rewrite" into "add a field + a renderer branch."

### F2. The **visual model is decoupled from the hitbox** — from day one
The collision/hitbox is fixed engine geometry (`player.rs` capsule/AABB); the cosmetic descriptor only ever drives **visuals**. No cosmetic — skin, overlay, future 3D model, cape, hat — can change the hitbox. This (a) neatly designs out the exact problem that made Mojang freeze custom models (cosmetic geometry ≠ gameplay advantage), and (b) keeps us **server-authoritative**: the server validates movement against the fixed hitbox, indifferent to whatever the client is wearing. Phase 1 cost: keep the proportion changes (§1.1) purely in the *visual* model; verify the hitbox/eye-height are separate constants the cosmetic path can't touch.

### F3. Cosmetics are **Stash assets keyed by persona**, typed by kind
`kind: "cosmetic"` (a descriptor wrapping the skin) now; the same kind grows to carry model/cape/emote fields later — all the same per-persona encrypted-and-portable store. This is what makes "owned by you, carries across our games" real, and (when Bitcoin lands) what a creator *sells* you: a signed cosmetic asset your persona then owns. No new transport ever needed — it's the primitive we already built.

> These three are the entire "build it so we can facilitate something better" ask. Everything else in this spec is the concrete round-one work that sits on top.

---

## The prize — and exactly which phase delivers each piece

Owner's stated prize (2026-06-02): *"a signed-in user uploads a local file → it becomes their skin → they see it on their own hand/avatar immediately, AND other players (or spectators) see them wearing it in multiplayer."* That is **three pieces, and they do NOT all land in one phase** — be honest about this:

| Piece | What it means | Phase | Status / why |
|---|---|---|---|
| **A. Upload from a local file → saved** | file picker → 64×64 PNG → stored per-persona, follows you across devices | **Phase 2** | needs upload UI + Stash `kind:"cosmetic"` |
| **B. You see your OWN skin** | your avatar + first-person hand show the uploaded skin immediately | **Phase 2** | local texture replace on the Phase-1 skin path |
| **C. OTHERS see your skin (multiplayer)** | remote players / spectators render you wearing it | **Phase 3** | **NOT trivial:** `protocol.rs::PlayerState` (the multiplayer wire struct) has **no skin field** — only x/y/z/yaw/pitch/health/held/anim/flags. Today other clients have no way to receive your skin. Needs: skin delivery (over protocol, or each client fetches the persona's published Stash manifest, Figura-style) + a **per-player skin texture array** (today all avatars share one texture) + the avatar shader selecting per-player. This is roughly as much work as Phases 0–2 combined. |

**Full prize = Phases 0 + 1 + 2 + 3.** Phase 0+1 is the foundation (standard avatar, descriptor seam, hitbox separation — no user-visible skin feature yet). Phase 2 is the "solo prize" (A+B). Phase 3 is the "others see you" payoff (C). Owner wants the full prize; Phase 3 is the headline-but-hardest part and gets its own plan written against the merged Phase-1/2 code.

---

## Current state (verified 2026-06-02)

- **Model:** `entity_model.rs` `PLAYER_MODEL` (LazyLock, ~line 1671) — 6 `ModelPart` cuboids:
  - Head `size 0.5×0.5×0.5`, origin y=1.575
  - Body `0.5×0.7×0.25`, origin y=0.975
  - Arms `0.25×0.7×0.25` at x=±0.375
  - Legs `0.25×0.625×0.25` at x=±0.125
  - (World units; the *ratios*, not absolute sizes, are what matter for the standard.)
  - **2026-07-24:** body/legs restacked **flush** (legs [0, 0.625], body [0.625, 1.325],
    head [1.325, 1.825]) — the original table buried the leg tops 0.125 and the torso top
    0.05 inside their neighbours, and the coplanar buried faces z-fought under custom
    skins (report `4519351d`, "overlap at the body and the legs"). Visible silhouette is
    unchanged. Parts may touch at joint planes but must never overlap in volume — pinned
    by `entity_model::tests::player_model_parts_do_not_interpenetrate`; `skeleton::BIPED`
    and `skin_hit::BOXES` transcribe the same table (sync-tested).
- **Texturing:** 6 procedural layers `TEX_PLAYER_HEAD_FRONT/SIDE/TOP` (201–203), `TEX_PLAYER_BODY` (204), `TEX_PLAYER_ARM` (205), `TEX_PLAYER_LEG` (206), generated in `texture_gen.rs` (`gen_player_head_front()` etc., ~line 3177).
- **Vertex format:** `mesh.rs` `Vertex { position, normal, tex_layer: u32, uv: [f32;2], light }` — a per-vertex **texture-array layer index** + UV. (Crucially: UVs already exist per-vertex; the model just doesn't use sub-rect UVs today.)
- **Quad UVs:** `entity_model.rs::push_textured_quad` (~1507) assigns each face the **full 0..1 UV of its own layer** — i.e. one image per face, no sub-rectangles.
- **Texture array:** `renderer.rs` (~line 303) — a single D2 array, **`tex_size = 16u32` hardcoded**, `depth_or_array_layers = texture_count()`. Every layer is 16×16. Bound once as `texture_bind_group` (~line 152), shared by block + entity + player pipelines.
- **Avatar render:** `game_loop.rs` (~line 5503) appends remote-player avatars via `build_player_avatar_vertices` into a shared `entity_verts` buffer drawn with `entity_pipeline`. **All avatars currently share the one default skin** (per-player skin is DEFERRED — noted at `entity_model.rs:121`, would need per-player texture binding or a per-vertex channel).
- **Spec of record:** `docs/superpowers/specs/2026-05-24-player-avatars-viewmodel-design.md`.

---

## Phase 0 — the architectural decision: where a 64×64 skin lives

A Minecraft skin is **one 64×64 RGBA image**; faces map to sub-rectangles (e.g. head-front = px (8,8)–(16,16); the limbs/body each have front/back/sides/top/bottom tiles; a second "overlay" set sits at a different region for hat/jacket/sleeves). Our renderer can't represent that today because every texture-array layer is 16×16 and faces use full-face UVs.

**Three options. The spec recommends Option B.**

- **Option A — Tile the skin into the existing 16×16 array.** Slice the 64×64 into many 16×16 chunks, push as new layers, keep full-face UVs. *Rejected:* a skin face is 8×8 or 8×12, not 16×16 — doesn't tile cleanly; loses the overlay layer; explodes layer count per player; fights the format. Only "reuses existing code" superficially.
- **Option B (RECOMMENDED) — A dedicated 64×64 player-skin texture + sub-rect box UVs.** Add a *second* texture binding for the avatar: a 64×64 (RGBA) 2D texture (single skin for the default; later a 64×64 **array**, one layer per visible player). The avatar pipeline samples it with proper box-unwrap sub-rectangle UVs. Block/mob rendering is untouched (still the 16×16 array). This is how the format actually works and makes BYO-skins drop-in.
- **Option C — Make the whole entity array 64×64.** Upscale all entity textures to 64×64. *Rejected:* wasteful (mobs don't need it), bigger GPU footprint, and still needs sub-rect UVs anyway — all of B's work plus bloat.

**Option B specifics:**
- New texture: `player_skin_texture` (64×64 RGBA), its own `wgpu::Texture` + view + bind group (or extend the entity bind group with a second binding). Default content = our original skin (Phase 1). Phase 2 makes it a **64×64 D2 array** indexed per visible player.
- New UV path: the avatar's `push_textured_quad` calls pass an explicit **UV rectangle** per face (the standard box-unwrap coordinates) instead of `[0..1]`. Add `push_skin_quad(verts, normal, a,b,c,d, uv_min, uv_max)` or extend the existing fn with optional UV bounds. `tex_layer` for skin verts indexes the skin texture/array, not the block array — so the avatar likely needs **its own pipeline or a shader branch**; simplest is a dedicated `avatar_pipeline` bound to the skin texture.
- WGSL: an avatar fragment shader sampling the 64×64 skin (with the overlay layer alpha-composited — see Phase 1.4).

This is the bulk of the renderer work and the honest reason this is "medium," not "easy."

---

## Phase 1 — Standardise the avatar (no skins feature yet)

Goal: the avatar is a clean, standard-64×64-layout humanoid wearing **our own original default skin**, **rendered from a cosmetic descriptor** (F1) with a **hitbox decoupled from visuals** (F2). Ships as a self-contained improvement; no upload, no per-player skins, no Stash dependency — but the descriptor seam is in place so Phase 2/3 are additive.

### 1.0 Cosmetic descriptor + hitbox/visual separation (F1, F2 — do first)
- New `cosmetics.rs` (or in `entity_model.rs`): `CosmeticDescriptor { version: u8, /* round one: */ skin: SkinSource }` where `SkinSource` is `Default | Bytes(Vec<u8>)` (a 64×64 PNG). Add a `CosmeticDescriptor::default()`.
- The avatar render path takes a `&CosmeticDescriptor` and chooses the skin texture from it — even though round one only ever passes `default()`. Do NOT let the avatar read a hardcoded skin layer directly; route through the descriptor.
- **Hitbox audit:** confirm the player collision capsule/AABB + eye-height live as fixed engine constants (`player.rs` / `player_intent.rs`) and that NOTHING in the avatar/cosmetic path feeds them. Add a short doc-comment at both the hitbox constants and the avatar builder stating the invariant: *cosmetics are visual-only; they never touch the hitbox.* This is the cheap insurance that designs out Mojang's custom-model-cheating problem.
- Unit test: `CosmeticDescriptor::default()` produces the default skin; a `Bytes(..)` descriptor selects custom (can stub the texture side). Assert the avatar builder signature takes the descriptor.

### 1.1 Proportions → standard ratios (visual model only)
In `PLAYER_MODEL` (`entity_model.rs`), adjust `size`/`origin`/`pivot` so the head:body:limb ratios match the standard humanoid (head 8, body 8w×12h×4d, limbs 4×12×4, in skin-pixel units; scale to our world-unit player height). **These are VISUAL proportions only (F2)** — they must not change the collision hitbox or eye-height; if total visual height shifts, the hitbox constants stay put unless deliberately re-tuned as a separate, gameplay-reviewed change. Update the `player_model()` unit tests that assert `parts.len()==6` and pivots.

### 1.2 64×64 skin texture path (Option B)
- `renderer.rs`: create the 64×64 `player_skin_texture` + view + sampler + bind group; load the default skin bytes.
- New `avatar_pipeline` (or extend `entity_pipeline` with the skin bind group) using an avatar WGSL that samples the 64×64 skin.
- `game_loop.rs` avatar render (~5503): draw avatars with the avatar pipeline + skin bind group instead of the shared 16×16 entity texture.

### 1.3 Box-unwrap UVs
- `entity_model.rs`: replace the avatar's full-face quads with sub-rect UVs per face per part, matching the standard 64×64 layout. Define the layout as named constants (a `skin_uv.rs` table: for each part+face, the `(u0,v0,u1,v1)` in 0..1 over the 64×64). Unit-test that all rects fall within bounds and don't overlap wrongly.
- Remove/retire `TEX_PLAYER_HEAD_FRONT..LEG` (201–206) from the 16×16 array (or leave as dead fillers to keep later layer indices stable — mirror the "retired layers" pattern already used at `entity_model.rs:28`). **Decide explicitly; don't silently shift indices.**

### 1.4 Overlay (hat/jacket) layer — do it now, not later
The standard skin has a second "overlay" set (hat over head, jacket over body, sleeves/leggings) rendered as a slightly inflated second box with alpha. Without it, hair/glasses/clothing in most skins look flat or missing. Render each part twice (base + inflated overlay) sampling the overlay UV region, with alpha blend / alpha-cutout. This is extra geometry (6 parts → up to 12) but is what makes real skins look right.

#### Exact inflate values (CORRECTED 2026-07-25 — do not guess these)

The inflate is **per side** and **differs by part**. Minecraft grows the hat box by
1 px total (0.5 px per side) and the jacket / sleeve / trouser boxes by 0.5 px
total (0.25 px per side). One skin pixel is 1/16 of a block:

| Part | Minecraft growth | Per side | World units |
|---|---|---|---|
| Head (hat) | 1 px total | 0.5 px | **0.03125** |
| Body, arms, legs | 0.5 px total | 0.25 px | **0.015625** |

> **The "~+0.5px" wording above was the bug.** It was read as one flat value and
> implemented as `0.03` for every part — correct for the head by luck, but nearly
> **double** Minecraft's for the torso and limbs, so imported Minecraft skins
> rendered with a chunkier jacket and sleeves here than they have in Minecraft.
> Corrected in v0.2.16. If you are re-deriving this, the source of truth is
> `skin_pose::overlay_inflate(part)` — never a literal.

**Workshop edit-time exaggeration.** The blown-up painting mannequin (and only it)
uses `skin_pose::EDIT_INFLATE = 0.0625` — a full pixel per side. At the ×4 blow-up
the Minecraft-exact sleeve gap is ~0.06 world units, which reads as zero on screen
and makes the clothes layer impossible to aim at deliberately. The worn avatar and
every exported PNG stay Minecraft-exact so skins look identical here and there.

#### Model-space offsets must be rotated (Workshop limbs-apart)

The Workshop can separate the limbs so the buried faces can be painted (8 of the
36 faces are otherwise unreachable — inner arms, inner legs, leg tops, body top and
bottom). The part builder applies `R_y(yaw)` **then** translates by `entity_pos`, so
a model-space displacement **must be rotated by the same yaw** before it is added to
the position. Applying it directly to `entity_pos` separates the limbs along *world*
axes while the hit-test — which un-rotates the ray via `world_ray_to_avatar_model`
and works purely in model space — expects them to separate along the avatar's own.
That divergence makes the crosshair land somewhere other than the limb.

**Render and hit-test both derive from `skin_pose`** (`part_offset` / `part_box`)
for exactly this reason; the box table used to be duplicated in `skin_hit.rs`.
Never hardcode a box, an offset or an inflate anywhere else.

### 1.5 Author our default skin
Create an **original** 64×64 default skin PNG (our art — see §IP). Ship it as the bundled default in `player_skin_texture`. Optionally keep a procedural fallback.

### 1.6 Verification
- `check.sh` green; avatar renders correctly (face forward, limbs animate, held item still attaches — `held_item_model.rs` unaffected).
- Visual playtest (the irreducibly-GPU part): avatar looks right from all angles, walk/crouch/jump anims intact, overlay layer shows.

---

## Phase 2 — Bring-Your-Own Skins (rides on Stash)

Goal: a player uploads a standard 64×64 PNG; it becomes their skin and follows their persona to any device via Stash.

### 2.1 Upload + validation (web layer)
- Lobby/in-game UI: "Choose your look → Upload a skin (64×64 PNG)". A file picker (mirror the existing `js_pick_world_file` import flow in `wasm_save.rs`).
- Validate client-side: PNG, exactly 64×64 (or 64×32 legacy → reject or upscale; recommend 64×64 only for v1), RGBA. Reject oversize / wrong dims with a kid-friendly message.

### 2.2 Store the cosmetic descriptor in Stash (`kind: "cosmetic"`)
- Store the **`CosmeticDescriptor`** (F1/F3), not a bare skin blob — so Phase 3 fields (model, cape, emotes) extend the same asset with no migration. Round one the descriptor just wraps the 64×64 PNG. Save via `AxeCloud.save` with a cosmetic kind (extend the bridge: `axenstax_cloud_save_cosmetic(descriptorBytes)` → `stash.save('cosmetic', 'avatar', descriptorBytes)`), list/restore the same way. NIP-44 encrypt-to-self like worlds.
  - *(For the very first cut you may store the raw PNG under `kind:"skin"` if simpler, but prefer the descriptor envelope from the start — it's the F1/F3 payoff and avoids a re-key later.)*
- On sign-in, fetch the persona's cosmetic descriptor from Stash (if any) and apply it; fall back to default when none / cloud off / offline (IndexedDB local copy as the fast path, mirroring worlds).

### 2.3 Apply the skin to YOUR OWN avatar + hand (prize pieces A+B)
- Decode the uploaded 64×64 PNG → replace the local player's `skin_texture` (Phase 1 made this the skin path; single 2D texture replace).
- Apply it to BOTH the third-person avatar AND the first-person viewmodel hand (`viewmodel.rs` reuses `player_model()`'s right arm — it must sample the same uploaded skin so "you see your hand straight off" works).
- **Phase 2 scope = local player only** (you see your own skin everywhere you appear). Other players seeing it is Phase 3 — do NOT try to cram it here; the wire format isn't ready (see §The prize, piece C).

### 2.4 Verification (Phase 2 = solo prize)
- Upload a 64×64 PNG → your avatar AND your first-person hand show it immediately. Sign in on another device (same persona) → skin restored from Stash. Wrong-size file → friendly rejection. No skin → default. (E2E needs browser + a capable signer — same playtest boundary as cloud save.)

---

## Phase 3 — Others see your skin (prize piece C; the headline payoff)

Goal: in multiplayer, remote players and spectators render each player wearing *their own* uploaded skin. This is the largest phase; gets its own task-by-task plan written against the merged Phase-1/2 code. Sketch of what it requires:

### 3.1 Skin delivery to other clients
Two viable routes (decide in the Phase 3 plan):
- **(a) Manifest-fetch (Figura-style, preferred — no new wire bloat):** each persona's cosmetic descriptor is already a published Stash asset (Phase 2, F3). A client seeing player X looks up X's persona npub → fetches X's cosmetic from the relay/Blossom → decrypts? *No* — for OTHERS to read it, the skin must be **readable by them**, so a *shared* cosmetic (skin) is published NOT encrypt-to-self but either plaintext or to a known cosmetic kind. **Design note:** worlds are encrypt-to-self (private); a *skin you wear in public is inherently public* — so Phase 3 publishes the skin as a public, persona-signed asset (a separate kind from the private world vault), keyed by the persona npub. This is a real model decision: skins are public-by-nature, unlike world saves.
- **(b) Over the protocol:** add skin (or a skin hash/URL) to the join/player-state path so the server relays it. Simpler to reason about for LAN, but bloats the wire and couples to our server; (a) is more aligned with the decentralised posture.

### 3.2 Per-player skin texture array (renderer)
The Phase-1 single 64×64 `skin_texture` becomes a **64×64 D2 texture array**, one layer per visible player (+ a default layer). The avatar pipeline selects the layer per player (a per-draw uniform or a per-vertex layer index). Bound on `max_texture_array_layers` (already pinned in renderer.rs) — cap visible custom skins, fall back to default beyond the cap.

### 3.3 Identity binding
A player's skin is keyed to their **persona npub** (the same identity that owns it in Stash), sourced from the verified auth path (`PlayerState`/join carries the pubkey per the multiplayer-identity design, Spec 1 Phase 4) — never a client-asserted name. Ties cleanly into the existing Signet identity work.

### 3.4 Verification
Two real clients, two personas, each with a different uploaded skin → each sees the other wearing the correct skin; a player with no custom skin shows default; exceeding the skin cap falls back gracefully. (Needs two browsers + capable logins + a running session — the multiplayer playtest boundary.)

---

## IP posture (important — read before authoring art)

- **The 64×64 UV layout is a functional interoperability spec, not protected expression.** Supporting it is equivalent to a program reading a common file format. No Minecraft IP issue in *consuming/producing* the format.
- **DO NOT** ship Minecraft's default skins (Steve/Alex), and **do not** bundle, scrape, or pre-load any third-party skin library (Skindex/NameMC/SkinDeck etc.). That art is owned by Mojang or individual creators.
- **Ship our own original default skin**, authored to the standard layout.
- **User uploads** are user-generated content: the liability for an infringing uploaded image sits with the uploader, handled via terms + takedown — the same posture every skin site already runs. Frame the feature as **"bring your own skin,"** never "browse skins."
- Avoid Minecraft trademarks in copy ("skins" is generic and fine; don't imply Minecraft compatibility/endorsement in marketing).

---

## Effort estimate (honest)

- **Phase 1:** medium. The renderer work (64×64 skin texture path, sub-rect box UVs, overlay layer, avatar pipeline/shader) is the real cost — call it the bulk. Proportion tweaks + default-skin art are smaller. Compile-verifiable here; correctness is GPU/visual → playtest.
- **Phase 2 (solo prize, A+B):** small-to-medium once Phase 1 + Stash exist — mostly upload UI + `kind:"cosmetic"` descriptor Stash wiring + local texture replace (avatar + viewmodel hand).
- **Phase 3 (others-see-you, C):** **large** — roughly Phases 0–2 combined. Per-player skin texture array + skin delivery to other clients (public persona-keyed asset, NOT encrypt-to-self) + identity binding. The headline payoff and the hardest part.

**Full prize (owner's goal) = Phases 0+1+2+3.** Recommended build order: 0+1 (foundation) → 2 (you see your own) → 3 (others see you). Each phase is shippable/testable on its own.

---

## Phase 4+ — beyond the prize (sketch, separate specs later)

Not built here; sketched so Phases 1–3 don't foreclose them. All build on the **same cosmetic descriptor (F1)**, the **hitbox/visual separation (F2)**, and **Stash persona assets (F3)** — each is "add a descriptor field + a renderer branch + (maybe) delivery," not a rewrite.

- **Custom 3D geometry** (the #1 community want) — descriptor carries a model (Blockbench-style box list); renderer builds from it instead of the fixed humanoid; **hitbox stays the fixed capsule (F2)** so cosmetic geometry never affects fairness — designing out Mojang's exact blocker. Watch the performance budget (instancing helps).
- **Overlays / capes / hats / wings** — additional descriptor layers; capes are a back-quad the community explicitly wants and Mojang gatekeeps.
- **Emotes** — descriptor-referenced animation clips; sync over the same rails as multiplayer cosmetics.
- **Creator-sold cosmetics (with Bitcoin)** — a creator signs a cosmetic asset; a player buys it **directly over Lightning** (no marketplace cut, not a money transmitter — our existing posture) and it lands in their persona's Stash as an owned, portable, cross-game asset. This is the strategic payoff Mojang structurally cannot match. Needs: a cosmetic-as-purchasable-asset format, a buy flow, and **UGC moderation/terms** (infringing-model risk grows the moment cosmetics are sellable).

## Deferred / out of scope (round one)

- ~~**64×32 legacy skins, slim/"Alex" 3px-arm variant**~~ — 64×32 legacy import shipped
  (`mc_import::expand_legacy_skin`); **Slim shipped 2026-09-06**. `skin_uv::ArmModel`
  (`Classic` | `Slim`) is threaded explicitly through the UV tables, `skin_pose::part_box`,
  `skin_hit`, `skin_grid`, the mesh builders and the thumbnails, and stored per wardrobe
  entry (`SkinEntry.arm_model`, blob v3) — a wardrobe can hold both at once.
  The three differences, and nothing else changes:
  (1) **Box** — the arm is 3/16 wide instead of 4/16, the whole pixel coming off the OUTER
  edge so the inner edge stays flush with the torso, and the arm (box *and* shoulder pivot)
  drops 0.5 px, matching Java `PlayerModel`'s y 2.5-vs-2 rotation point.
  (2) **Atlas** — the front/back/top/bottom arm tiles shrink to 3 px and every tile after the
  front slides 1 px left (right arm: front `(44,20,3,12)`, inner `(47,20,4,12)`; left arm:
  front `(36,52,3,12)`, outer `(39,52,4,12)`); the two ±X side tiles keep their 4-px width
  because they show the arm's unchanged depth.
  (3) **Nothing else** — head, body and legs are byte-identical, which is why one PNG is
  valid for both models and only the per-entry setting differs.
  Still deferred: **remote players** are always drawn Classic — the arm model is not on the
  wire (only `skin_key` is), the same limitation that already leaves remote peers without
  custom skins.
- **A skin *browser*/library** — deliberately not done (IP). BYO upload only; a *creator marketplace* is the Phase 3 Bitcoin play, not a bundled library.

---

## File-touch map

| File | Phase | What |
|------|------|------|
| `game/engine/src/cosmetics.rs` (new) | 1 | `CosmeticDescriptor` (F1) + `default()` + tests; the seam Phase 2/3 extend |
| `game/engine/src/entity_model.rs` | 1 | `PLAYER_MODEL` *visual* proportions; box-unwrap UV quads; overlay parts; avatar builder takes `&CosmeticDescriptor`; retire `TEX_PLAYER_*` |
| `game/engine/src/skin_uv.rs` (new) | 1 | 64×64 box-unwrap UV-rectangle table + tests |
| `game/engine/src/player.rs` / `player_intent.rs` | 1 | hitbox/eye-height stay fixed engine constants (F2); doc-comment the cosmetics-are-visual-only invariant |
| `game/engine/src/renderer.rs` | 1 | 64×64 `player_skin_texture` + view/sampler/bind group; avatar pipeline; (Phase 2) → D2 array |
| `game/engine/src/*.wgsl` (avatar shader) | 1 | sample 64×64 skin; overlay alpha-composite |
| `game/engine/src/game_loop.rs` | 1 | avatar render uses avatar pipeline + skin bind group |
| `game/engine/src/texture_gen.rs` | 1 | original default 64×64 skin (replaces `gen_player_*` 16×16 layers) |
| `tools/sites/game/static/cloud.js` | 2 | `kind:"cosmetic"` descriptor save/list/restore extern wrappers |
| `game/engine/src/wasm_save.rs` | 2 | skin upload (pick file) + cosmetic-descriptor cloud externs/wrappers |
| cosmetic upload UI (lobby/in-game) | 2 | file picker + 64×64 validation + "your look" surface |

---

## Memory-rule check

- **IP / no-Mojang-art:** ship only our originals; BYO upload; format is interop-only; UGC moderation planned before cosmetics are sellable. ✅
- **`project_shared_infra_strategy`:** cosmetics ride Stash (`kind:"cosmetic"`) as generic per-persona assets — the cross-game-cosmetics play IS the shared-infra strategy realised. ✅
- **`project_stash_locket_save_architecture`:** cosmetics are the first non-world Stash asset — proves the primitive beyond worlds. ✅
- **`project_economies_vision` / Bitcoin posture:** Phase 3 creator-sold cosmetics = direct Lightning, no marketplace cut, not a money transmitter — consistent with existing economy/settlement model. ✅
- **UK English**; **npub never hex** in any user-facing surface. ✅
