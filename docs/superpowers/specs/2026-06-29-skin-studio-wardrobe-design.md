# Skin Studio & Wardrobe — Design Spec

- **Date:** 2026-06-29 (updated 2026-06-30)
- **Status:** Live — full wardrobe + in-Workshop painter shipped (2026-06-30)
- **Authors:** Staxolottle (direction) + Claude (design)
- **Supersedes/extends:** the avatar cosmetics in `docs/superpowers/specs/2026-05-24-player-avatars-viewmodel-design.md`
- **Spec maintenance:** per CLAUDE.md, this is the source of truth for the skin editor + wardrobe. Update it when the design changes or a bug reveals a wrong assumption.

---

## 1. Goal

Let a player **create, edit, organise, and reuse multiple avatar skins** entirely in-game, including the **outer (overlay) layer**, and move skins **in and out of the standard Minecraft format**.

Concretely, a player can:

- Keep a **wardrobe** of many named skins, pick which one they wear, and add/duplicate/rename/delete them — like a real wardrobe of outfits.
- **Paint** a skin per-pixel by clicking on a 3D mannequin in the **Workshop** (base + outer layers).
- **Import** a skin four ways: new blank, paint from scratch, upload a 64×64 PNG, or **pull from a Minecraft username**.
- **Download** any skin as a launcher-ready **64×64 Minecraft PNG**, on web and native.
- Persist the wardrobe per-platform: **web = local only**, **native = local + opt-in Stash**.

## 2. Current state (what exists today)

| Capability | State | Reference |
|---|---|---|
| Two-layer 64×64 avatar skin rendering (base + overlay, all 6 parts) | ✅ live | `skin_uv.rs` (`base_faces`/`overlay_faces`), `entity_model.rs::build_skin_part_vertices`, `OVERLAY_INFLATE = 0.03` |
| Single-skin cosmetic descriptor | ✅ live | `cosmetics.rs::CosmeticDescriptor { version, skin: SkinSource::{Default, Rgba64} }` |
| PNG decode → 64×64 RGBA (upload) | ✅ live | `cosmetics.rs::decode_skin_64` |
| Skin content hash for multiplayer identity | ✅ live | `cosmetics.rs::skin_key` (FNV-1a, 0 = default) |
| "Your look" panel: upload PNG / 5 colour presets / reset | ✅ live | `menu.rs` (~4373), `game_loop.rs` (~5973) |
| Per-identity skin save — **web** | ✅ live | `wasm_save.rs::cosmetic_save_wasm` / `cosmetic_load_wasm` (localStorage per-pubkey or `local` ns + Stash best-effort) |
| Per-identity skin save — **native** | ❌ TODO | save code is `#[cfg(target_arch = "wasm32")]`-gated |
| Workshop per-pixel paint (blocks in-world; mobs via 2D panel) | ✅ live | `workshop.rs`, `workshop_painter.rs::paint_cell`, `override_registry.rs::AuthoredFaces` |
| 3D click → texel hit-testing at any scale | ✅ live | `raycast.rs::pick_cell_in_cage`, `raycast.rs::face_texel` |
| Symmetry mirroring while painting | ✅ live | `workshop.rs::symmetric_cells` |
| **No** in-game skin editor, **no** wardrobe, **no** zoom in edit mode, **no** Minecraft import/export | ❌ | — |

**Our avatar is the Minecraft Classic ("Steve") model: 4px-wide arms**, confirmed by the `skin_uv.rs` header ("Standard Minecraft 64x64 'classic'") and the limb dims (`4x12x4`). This is the model toggle a player picks in their launcher after downloading our PNG.

## 3. Non-goals (v1)

- Capes, emotes, 3D model parts, animated cosmetics. (The descriptor is designed to add these as fields later — out of scope now.)
- Editing *other players'* skins.
- Two-way sync of anything with Mojang. The Minecraft path is **download only** (see §8.4).
- Gamepad support for the painter (parked, per current platform priority: PC mouse first, touch second).

## 4. Two surfaces

The feature lives in two places with a clean split:

- **"Your look" (pause menu) = the Wardrobe.** Grid of all the player's skins (thumbnails). Actions: **Wear**, **New (blank)**, **Edit** (→ enters the Workshop painter on that entry), **Duplicate**, **Rename**, **Delete**, **Import** (PNG file / Minecraft username), **Download** (Minecraft PNG). This is where "all available in your look" is satisfied.
- **Workshop = the Painter.** Where blank-creating and editing actually happen: a mannequin of your equipped avatar stands in the Workshop; aim the **Bellows** at it, hold right-click to blow it up (×4), fly around it, and paint per-pixel with dyes. **P** pins → saves back to the wardrobe entry and deflates.

Discovery flow: "Your look" → **Edit**/**New** teleports into the Workshop with the mannequin already blown up and locked on that entry; pinning (P) writes back and deflates.

## 5. Data model

### 5.1 Wardrobe wraps the existing descriptor

Today persistence holds one skin. We wrap it:

```
SkinWardrobe {                   // runtime struct (skin_wardrobe.rs)
    entries: Vec<SkinEntry>,
    equipped: SkinId,            // which entry the player is wearing
    next_id: SkinId,             // monotonic id allocator (survives reload)
}

SkinEntry {
    id: SkinId,                  // stable local id
    name: String,                // "Skin 1" default; user-renamable
    source: SkinSource,          // Default | Rgba64(16384 bytes) — reuses cosmetics.rs
    minecraft_handle: Option<String>,  // display only; set if imported from a MC username (§8.4)
    minecraft_uuid: Option<String>,    // the stable key for Refresh; resolved once on import (§8.4)
    created: u64,
    modified: u64,
}
```

**Versioning is a persistence concern, not a runtime field.** The on-disk/blob form is a separate `StoredWardrobe { version: u8, entries, equipped, next_id }` (with a `BLOB_VERSION` constant) that the serialization layer maps to/from the runtime `SkinWardrobe` — so the runtime struct carries no `version`. `next_id` is repaired on load against a stale/corrupt blob so a reloaded wardrobe can never hand out a colliding id.

### 5.2 The equipped entry derives the live descriptor

`Wardrobe::active_descriptor() -> CosmeticDescriptor` returns the equipped entry's `source` wrapped in the existing `CosmeticDescriptor`. **The renderer and multiplayer hash-sync keep consuming `CosmeticDescriptor` unchanged** — the wardrobe is purely a storage layer above it. This keeps the blast radius tiny.

### 5.3 Migration

Any previously-stored single skin loads as **wardrobe entry #1, equipped**. No data loss. Native has no prior persistence, so nothing to migrate there.

### 5.4 No `WorldSave` change

The wardrobe is **per-identity**, exactly like cosmetics are now — even though painting happens inside the Workshop world, **pin/save never touches that world's `WorldSave`**. No append-only byte-layout work; no save-format version bump.

## 6. The painter — Workshop blow-up flow

> **2026-06-30 (shipped):** The painter is no longer a standalone "Skin Studio" orbit panel. It is implemented as an extension of the **Workshop blow-up state machine** (the same Bellows → charge/lock/collapse/pin flow used for blocks and mobs). The standalone orbit panel (`draw_skin_studio`, `render_skin_preview`, `SkinPreviewTarget`, `SKIN_PREVIEW_ASPECT`) has been **retired** and deleted. §6.6 (orbit/zoom) is **N/A** — you navigate with standard Workshop fly.

### 6.1 Core paint loop

- A **mannequin of your equipped avatar** stands in the Workshop (skin array layer 0, wears live changes for free).
- Aim the **Bellows** at the mannequin + hold right-click → avatar **swells ×1→×4** through the charge animation → **locks** at full scale.
- While locked: crosshair left-click on the inflated figure → **world ray → `world_ray_to_avatar_model` → `skin_hit::ray_hit_avatar`** → `(part, face, frac_u, frac_v)` → `skin_uv::texel_for` → exact `(x, y)` pixel in the working 64×64 buffer → `skin_paint::pencil` / `erase` / `restore_from`.
- The working buffer is pushed to **skin layer 0** (`renderer::write_avatar_skin`) after every stroke → live preview is free and zero-cost (layer 0 is the worn layer; it tracks the working copy in real time).
- **P (pin)** = `skin_wardrobe::update_rgba` (overwrite the opened/worn entry) + `persist_skin_wardrobe` + `apply_local_cosmetic` → mannequin deflates, you wear the painted skin.
- Cancelling (release/sneak before P) → the session is dropped, `apply_local_cosmetic` restores the previous worn skin (layer 0 snaps back).

### 6.2 The reverse UV map (highest-risk logic)

`skin_uv.rs` gives the **forward** rects: `base_faces()` and `overlay_faces()`, each `[[UvRect; 6]; 6]` in part order `[head, body, arm_l, arm_r, leg_l, leg_r]` and face order `[+x, -x, +y, -y, +z, -z]`. The **inverse** (`texel_for(part, face, frac_u, frac_v, layer) → Option<(x, y)>` and `face_rect_px(part, face, layer) → Option<(x0, y0, x1, y1)>`) is implemented in `skin_uv.rs` and round-trip unit-tested.

#### 6.2a u DIRECTION — the box unwrap, not just the rect (bug fixed 2026-07-30)

Picking the right *rect* is only half the mapping; each face's **u direction** is pinned too, and getting it backwards is invisible on a symmetric skin.

The classic 64×64 layout is an **unrolled cube**. The head band runs `right | front | left | back` across atlas x=0..32 at y=8..16, and neighbouring tiles **share real box edges** — the right tile's u1 edge (atlas x=8) *is* the front tile's u0 edge. So walking forward around the model must walk forward along the atlas. Concretely, with the engine's `-Z = front`, `+X = character's right`:

| Face | u0 | u1 |
|------|----|----|
| +X (right side) | back (+Z) | front (−Z) |
| −Z (the face) | character's right (+X) | character's left (−X) |
| −X (left side) | front (−Z) | back (+Z) |
| +Z (back) | character's left (−X) | character's right (+X) |
| +Y / −Y | character's right (+X) | character's left (−X) |

**What was wrong:** `push_skin_quad` reused `push_textured_quad`'s corner→UV mapping verbatim (`a→u0 … b→u1`), which runs u the *opposite* way on every face. The result was internally coherent — so the painter, which derives `frac_u` from the same convention in `skin_hit::frac_uv`, agreed with it and in-engine painting looked correct — but it is the wrong chirality against Mojang's unwrap. Every face rendered mirrored about its own vertical axis: **the sides of the head read back-to-front** on an imported Minecraft skin (reported by the owner), text on the face came out reversed, and exports were mirrored.

**The fix:** `push_skin_quad` maps `a→u1, b→u0, c→u0, d→u1` (vertex order and winding unchanged), and `skin_hit::frac_uv` flips u on all six faces to stay its exact inverse. `mirror_hit` is unaffected — a left/right mirror still swaps the ±X faces and maps `u → 1−u` under either chirality.

Pinned by `entity_model::tests::skin_faces_follow_the_minecraft_box_unwrap`, which asserts atlas continuity across the `right|front` and `front|left` seams. That test is rotation-proof (it compares the u each face carries on their shared box edge), so it holds regardless of how the avatar is yawed in world space.

**Not migrated:** skins hand-painted in the Workshop before this fix were stored under the old chirality, so each of their faces now renders mirrored. Imported Minecraft skins and all future painting are correct.

The **world → model ray transform** (`workshop::world_ray_to_avatar_model`) inverts exactly what the renderer applies — translate by −`WORKSHOP_MANNEQUIN_POS`, inverse-rotate about Y by −(yaw+π/2), unscale by `AVATAR_BLOW_UP_SCALE`. The round-trip is pinned by a dedicated unit test (`world_ray_round_trips_to_a_front_face_hit`).

### 6.3 Mannequin render

- **Resting mannequin:** `entity_model::build_player_avatar_vertices` at layer 0, fixed position `WORKSHOP_MANNEQUIN_POS`, yaw `WORKSHOP_MANNEQUIN_YAW` (π/2, front faces +Z toward spawn). Hidden while a blow-up project is active.
- **Blown-up mannequin:** `entity_model::build_workshop_avatar_vertices` — same builder, scaled about the feet anchor by the animated `BlowUp::scale()` factor (1→`AVATAR_BLOW_UP_SCALE`). Renders via the avatar skin pipeline (not the block/entity pipeline).
- Block/mob projects: `build_workshop_inworld_vertices` skips `WorkshopTarget::Avatar` (early continue) so the two pipelines never collide.

**2026-09-03 (bug fixed — Bellows aim highlight under-covered the limbs-apart pose):**
`workshop::pick_avatar_mannequin` (the coarse aim box behind the highlight and
the "mannequin vs aimed block" nearest compare) used to test against a
hand-typed, limbs-TOGETHER-only box (`AVATAR_AABB_MIN/MAX`, x ±0.5 / y
0..1.86 / z ±0.25). With the limbs separated (R), the arms swing out to
x≈±0.75 and the head lifts to y≈2.075, so the aim highlight silently
under-covered a separated mannequin even though the real per-pixel paint ray
(`skin_hit::ray_hit_avatar`) stayed accurate. Fixed by adding
`skin_pose::avatar_aabb(separation, inflate)` — the union of the same 6
`BOXES` / `part_offset` tables `part_box` (and so `ray_hit_avatar`) already
use, never hand-retyped — and giving `pick_avatar_mannequin` a new
`separation` parameter. Both call sites (the pre-blow-up aim-to-inflate check
in `game_loop.rs`) now pass `self.skin_paint.as_ref().map(|s| s.limbs_t)
.unwrap_or(0.0)`, the same value the paint ray-test and renderer use, so the
aim box always matches the pose actually on screen.

### 6.4 Tools (in-world bindings)

| Gesture | Action |
|---|---|
| **Left-click + held dye** | Paint the hit pixel with the dye's block colour (`dye_skin_color`) |
| **Left-click + empty hand** | Erase: Outer layer → alpha 0; Base layer → restore default-skin pixel |
| **G** | Eyedropper — read the hit pixel into `SkinPaintSession.picked_color` (overrides the held dye for subsequent strokes until the hotbar slot changes) |
| **M** | Toggle left/right mirror (`skin_uv::mirror_hit`) — paints the symmetric hit simultaneously |
| **V** | Toggle Base / Outer layer (`SkinPaintSession.layer`) — V is repurposed from Paint↔Sculpt (Sculpt is meaningless for skins; `Reshape` is never reached on the Avatar path) |
| **P** | Pin → write to wardrobe, wear, persist, deflate |
| Release / sneak+right-click | Cancel and restore the previously worn skin (no change saved) |

`Pencil(brush)`, eyedropper, erase, and mirror are implemented. Fill (flood-fill) and Undo/Redo from the session `undo` stack (cap 24) are wired; a per-stroke snapshot is pushed before each rising edge.

### 6.5 Outer (overlay) layer

- **V** toggles `SkinPaintSession.layer` between `SkinLayer::Base` and `SkinLayer::Overlay`.
- **Eraser on Outer** sets alpha = 0 (`skin_paint::erase`), so the base shows through.
- **Eraser on Base** restores the default-skin pixel (`skin_paint::restore_from` + `texture_gen::default_skin_rgba`). Base is never transparent.

#### 6.5a "Base is never transparent" is an INVARIANT, not just an eraser rule (bug fixed 2026-07-31)

**Bug (Axolittle, 2026-07-31):** "the back of the arms on the skins are
trasparent" — a see-through hole in a painted body.

**Root cause:** the rule above was enforced only on the *eraser* path. The
**eyedropper (G)** sampled the working buffer verbatim
(`picked_color = get_pixel(..)`), and every overlay texel of an unworn clothes
layer is `[0,0,0,0]` — so grabbing a colour off the bare clothes shell captured
a fully **transparent** colour. `picked_color` is cleared only when the held
hotbar dye slot changes, **not** when you toggle layer with V, so that
transparent colour survived the switch back to Base and `pencil` wrote alpha-0
texels straight into the body. `fs_avatar` cuts out on `alpha < 0.5`, so those
texels render as a hole rather than a colour.

**Correct approach:** enforce the invariant where it is actually violated — at
the *write*, not at one of the paths that reaches it. Every base-layer colour
now goes through `skin_paint::bind_to_layer(color, layer)`, which forces alpha
255 for `Base` and leaves `Overlay` alone (transparent there is legitimate —
that IS the clothes eraser). The eyedropper itself is unchanged: sampling a
transparent overlay texel is meaningful *on the overlay*.

**Repair of already-damaged skins:** `skin_paint::heal_base_opacity` forces
alpha 255 across all 36 base face rects, and runs when a skin is loaded into
the painter session — so a body punctured before this fix heals the moment it's
opened to edit, and Pin writes it back solid. Deliberately **editor-only**:
worn and imported skins are never rewritten behind the player's back. The one
behaviour implication: a Minecraft skin that *intentionally* uses transparent
base pixels (a "floating head" effect) becomes solid if you open it in the
painter. That is judged the right trade — the engine defines base transparency
as invalid (this section) — but it is a real, deliberate change, not an
oversight.

**General lesson:** a rule stated as "the eraser does X" is a rule about one
call site; a rule stated as "the buffer never contains Y" is an invariant. When
a second path to the same buffer appears (here: the eyedropper), only the second
form survives. Guard the write.

### 6.6 ~~Zoom / orbit~~ — N/A (retired)

The standalone orbit panel and its dedicated edit camera are **retired**. Navigation in the Workshop painter is standard Workshop fly (the Workshop is always creative + fly). There is no minimum-zoom clamp; `skin_hit::ray_hit_avatar` is called with the player's live crosshair ray, which is always from outside the avatar bounding boxes at the distances you fly.

A 2D precision panel (§4, §14 Phase 4 fallback) remains a possible future addition for fine pixel work; it is not in v1.

### 6.7 Session state (`SkinPaintSession`)

Carried on `GameState.skin_paint: Option<SkinPaintSession>`. Fields: `project_id`, `editing_id` (which wardrobe entry Pin writes back to), `buffer` (working 64×64×4), `layer`, `brush`, `mirror`, `picked_color`, `picked_slot`, `undo`. Created lazily when a locked avatar project first appears; dropped on pin or cancel.

## 7. Wardrobe operations

| Action | Behaviour |
|---|---|
| **New (blank)** | Adds a transparent/empty entry, opens it in Skin Studio |
| **Edit** | Opens the entry's 64×64 into the Studio; **Save overwrites that entry** |
| **Duplicate** | Forks an entry, so a copy can be tweaked without losing the original |
| **Rename** | Entries are named ("Knight", "Pirate"); auto-named "Skin N" |
| **Delete** | Removes an entry; cannot delete the last/equipped without first picking another |
| **Wear** | Sets `equipped` → flows to the live avatar + multiplayer via the existing `CosmeticDescriptor` path |

**Save semantics:** Edit **overwrites** the opened entry; **Duplicate** is how you fork. (Decided default — not "always create new".)

## 8. Import sources

A wardrobe entry can be created four ways:

1. **New blank** — empty 64×64, painted in the Studio.
2. **Paint** — same Studio, starting from blank or an existing entry.
3. **Upload a PNG file** — drop a 64×64 Minecraft PNG; decoded via the existing `decode_skin_64`; becomes a new entry. Works web + native.
4. **Import from Minecraft username** — see §8.4.

### 8.4 Import from Minecraft username (download only)

Pull a player's **current public skin** by typing their Minecraft handle. Public data; **no Microsoft/Mojang login required.**

**Mechanism (three public, unauthenticated calls):**

1. Username → UUID: `api.mojang.com/users/profiles/minecraft/<name>`
2. UUID → profile (base64 `textures` blob) → **skin URL + model type (Classic/Slim)**: `sessionserver.mojang.com/session/minecraft/profile/<uuid>`
3. Fetch the PNG → standard 64×64 → new wardrobe entry via `decode_skin_64`.

> **UUID-first (important):** step 1 (username → UUID) is Mojang's deprecated, rate-limited legacy endpoint; UUIDs are the supported key. So resolve name→UUID **once** at first import, store the UUID (§5.1), and have **Refresh re-pull by UUID** (steps 2–3 only). This dodges the fragile endpoint, survives the player renaming, and is what mature tools (Crafatar et al.) do.

> **Endpoint drift:** Mojang has been migrating services under Microsoft for years. These public profile/skin endpoints work as of writing, but the exact URLs must be **re-confirmed at implementation time** rather than treated as fixed.

> **Own proxy, not a third party.** CORS-enabled relays exist (Crafatar, vrc.lol, Minotar), but routing through them means depending on someone else's server — the thing this design forbids — and exposing players' handles to a third party. Our proxy talks **straight to Mojang**, borrowing their lessons (UUID-first, short cache TTL) without the dependency.

**Per-platform:**

| Platform | How |
|---|---|
| **Native** | Direct HTTP from the player's machine to Mojang (reqwest, already used elsewhere). We never see the username. |
| **Web/PWA** | Browser cannot call Mojang directly (no CORS headers; the PNG would be a tainted cross-origin image we can't read back). A small route on **our own game server** (`tools/sites/game/`, the FastAPI app already serving the PWA) — e.g. `GET /mc-skin?name=…` — performs the three calls server-side and returns the PNG. May cache briefly to stay well under Mojang's rate limit. |

**Download / refresh, not sync (one-way):**

- The entry **stores the resolved UUID + handle locally** (`SkinEntry.minecraft_uuid` / `minecraft_handle`) so nothing needs retyping and refresh is robust.
- A **Refresh / re-download** action re-pulls the *current* skin from Mojang **by UUID** — for when the player changed their skin in the real launcher and wants the new one here.
- This is strictly **download** (Mojang → us). We never push anything to Mojang. The word "sync" is reserved for the Stash feature (§10), which is our own service.

**Hard constraints (per owner):**

- **Never a runtime dependency.** The game, login, and play must work with Mojang/our proxy unreachable. Import is best-effort and on-demand; failure shows "couldn't reach Minecraft right now — try again" and changes nothing else.
- **No data retention.** The username is the player's own *public* handle, held locally on their device. Native never exposes it to us. The web proxy relays it transiently and **must not log or store it**. The skin returned is public. We therefore hold no one's personal data via this feature (see §9).
- **Legacy skins:** very old 64×32 skins fail `decode_skin_64`. v1 either converts 64×32→64×64 (fixed known mapping) or shows "this old skin needs updating in Minecraft first." Pick conversion if cheap; otherwise the friendly message. (Resolve during planning.)

## 9. Export (Minecraft PNG) — web + native

**Download → `<name>.png`**, a standard **64×64 Minecraft skin PNG**. Our layout already *is* the modern 64×64 humanoid format (§2), so export is "encode the buffer as PNG and hand it over."

| Platform | How |
|---|---|
| **Web** | Browser download: buffer → canvas → PNG blob → file in Downloads. |
| **Native** | Save-file dialog (rfd) / write to a known folder. |

In the launcher the player picks **Classic (Steve)** — our 4px-arm model — to match. Document this next to the download button.

Export is symmetric with the PNG import (§8, source 3), cheap, and the headline demo ("paint here, wear it in real Minecraft"), so it lands in **Phase 2** (§14), ahead of the import/Stash work.

## 10. Persistence by platform

The wardrobe is **per-identity**. Storage:

| | Storage |
|---|---|
| **Web** | **Local only.** Whole wardrobe in localStorage (per-pubkey when signed in, `local` ns on the anonymous taster). No Stash on web — consistent with the web-taster minimal-data posture. |
| **Native** | Local per-identity file **+ opt-in Stash**: **Upload** a single skin to Stash, or **Sync** the whole wardrobe to/from Stash. Both opt-in (player's choice, player's keys). Requires finishing the native cosmetic-persistence TODO (§2). |

"Sync" here = our own Stash, which genuinely can be two-way. This is a different mechanism from the Minecraft download (§8.4), which is one-way only.

### 10.1 Box-unwrap-fix blob migration (2026-09-03, v0.2.19)

Commit 712ec12c ("avatar faces follow the Minecraft box unwrap", v0.2.18,
2026-07-30) fixed `skin_uv.rs`'s `base_faces()`/`overlay_faces()` tables: the
u axis had run **backwards on every one of the 36 base + 36 overlay face
rects**, so every hand-painted skin from before that fix is stored mirrored
per-face relative to the Minecraft-standard layout and now renders mirrored
under the corrected renderer. `game/engine/src/skin_wardrobe_store.rs` runs a
**one-shot migration on load** to fix this in place:

- **Blob version bumped 1 → 2** (`BLOB_VERSION`). Loading a `version == 1`
  blob triggers the migration; loading `version == 2` (or any version other
  than 1) is a plain load — never re-migrated. A migrated wardrobe is
  written back as v2 the next time it's saved (`to_blob_bytes` always writes
  the current `BLOB_VERSION`).
- **Migration rule:** for each entry where `!is_default && mc_handle.is_none()
  && mc_uuid.is_none() && modified < UNWRAP_FIX_CUTOFF`, apply
  `skin_uv::mirror_every_face` — a pure horizontal flip of the texel columns
  *inside* every base + overlay face rect (rows untouched, atlas regions
  outside every rect untouched). `UNWRAP_FIX_CUTOFF = 1_785_436_892`, the
  committer-time (UTC seconds since epoch) of 712ec12c. `modified` is
  stamped by `game_loop::wardrobe_now()`, which returns **seconds** since
  epoch on both native (`SystemTime`) and web (`js_sys::Date::now() /
  1000.0`) — confirmed by reading that function, so the cutoff constant is
  in the same unit and needs no scaling.
- **`modified == 0`** (entries that predate timestamps entirely, e.g. ones
  created via the legacy-PNG `migrate_legacy_png` path) counts as **old** and
  is flipped — it necessarily predates the fix.
- **Left untouched:** the default entry (`is_default`, no pixels to flip);
  any entry imported from Minecraft (`mc_handle`/`mc_uuid` set) — those were
  already Minecraft-standard before and after the fix; any entry with
  `modified >= UNWRAP_FIX_CUTOFF` — the player already saw it under the
  fixed renderer (and may have hand-corrected it), so re-flipping it would
  be wrong.
- **Native safety net:** the first time a v1 blob is read from disk,
  `skin_wardrobe_store::load_from_path` copies the untouched original bytes
  to `<path>.v1.bak` (`std::fs::copy`, skipped if that backup already
  exists) *before* anything in the load path can overwrite `profile/skins.blob`.
  A copy failure only logs a warning — it never blocks the load.
- **Web has no backup.** The wardrobe blob lives in localStorage; there is
  no separate file to copy, so a web player has no automatic rollback if the
  migration heuristic is wrong for their case. Consistent with the
  local-only web posture in the table above.
- **Known limitation:** an entry **imported** from Minecraft but then
  **edited in the old (buggy) in-game painter** keeps its mirrored edits —
  the entry still carries `mc_handle`/`mc_uuid`, so it's indistinguishable
  from an imported-and-never-touched entry and is deliberately left alone by
  this migration. Not detectable from the stored data; would need a manual
  re-edit or re-import by the player.

## 11. Privacy & data protection

- The Minecraft handle is a **public identifier**, stored **locally** on the player's device; entering it is not us collecting sensitive data.
- **Native** import is a direct device→Mojang request; we never receive the handle.
- **Web** import passes the handle through our proxy **transiently only** — no logging, no persistence.
- All skins fetched are public; we redistribute nothing at scale (a player pulls their own skin).
- Import is **opt-in** and never on the critical path, so the running service has **no dependency** on a third party.
- Consistent with the broader posture in `docs/research/2026-06-21-uk-online-safety-gambling-crypto-landscape.md` and the web-taster decision (no login/Stash on web).

## 12. Multiplayer / rendering integration

Unchanged. The equipped entry derives a `CosmeticDescriptor` (§5.2); the existing `skin_key` hash drives identity diffing on the wire; remote players see a repaint/import through the path already used for uploaded skins. No protocol change.

## 13. Testing

- **Pure unit tests on the reverse UV map** (§6.2): for every base + overlay rect, `forward(part,face,uv) → texel → reverse → (part,face,uv)` round-trips; no two faces collide; every painted texel lands inside the correct atlas rect.
- **Flood-fill** stays inside its UV island (no bleed across parts).
- **Eraser** → alpha 0 on outer, opaque on base.
- **Save/load round-trip is byte-identical** on both web and native; pinned entry is a valid 16384-byte `Rgba64`; `skin_key` matches.
- **Wardrobe ops:** migration of a legacy single skin → entry #1 equipped; can't delete the last/equipped; duplicate produces an independent copy.
- **Export** produces a decodable 64×64 PNG that `decode_skin_64` round-trips back to the same bytes.
- **Minecraft import:** mock the three responses; assert a valid entry + stored handle; assert refresh re-pulls; assert all failure modes (unknown user, network down, 64×32 legacy) degrade gracefully and change nothing else.
- Integration via `TestHost` where feasible (enter studio, paint a known texel, save, assert wardrobe bytes).

## 14. Build phases (sequencing, not scope cuts — all of it ships)

1. **Wardrobe foundation + core paint, base *and* outer layer (PC/mouse).** Data model + migration + `active_descriptor`; "Your look" grid (wear / new / delete); open-in-Studio; Studio paint with **pencil, eraser, palette, orbit/zoom, basic undo, and the Base/Outer layer toggle (outer eraser → alpha, base ghosting)**; save-back. Persistence on **web + native local** (finishes the native cosmetic TODO). *(The outer layer is the headline ask and a small delta on the base machinery — it ships in the first usable version.)* **DONE (2026-06-30, Phase 1c)** — wardrobe foundation + grid + open-in-Studio + save-back + web/native local persistence all shipped; legacy single-skin loader retired; dead-code bridges removed. **Painter relocated to the Workshop blow-up flow (2026-06-30)** — the standalone Skin Studio orbit panel and `render_skin_preview` are retired; the painter is now the Workshop's Bellows → charge/lock → crosshair dye-paint → P-pin flow (see §6); "Your look → Edit / New" teleports into the Workshop and auto-inflates on the chosen entry.
2. **Richer tools + Minecraft export.** Fill (bucket), eyedropper, full RGB/HSV picker, redo + deeper undo, symmetry; wardrobe rename/duplicate; **Minecraft PNG export (web + native)**. **Minecraft PNG export DONE (2026-06-30)** — "Export to Minecraft ⬇" in the Studio + per-entry "Export ⬇", web Blob download + native rfd save, with the "wear it in real Minecraft" 3-step help; `cosmetics::encode_skin_png`. *Remaining Phase-2 polish (HSV picker, redo, symmetry) deferred.*
3. **Import + Stash.** PNG file **import** (both platforms); **Import from Minecraft username** (native direct + web proxy, UUID-first, refresh by UUID); native **Stash** upload/sync. **Minecraft-username import DONE (2026-06-30)** — "Bring in a Minecraft skin" card/dialog; web via our own no-log/transient/cached(300s)/rate-limited(10·IP·60s)/Mojang-only `GET /mc-skin` FastAPI proxy, native via `ureq` direct; UUID-first + refresh-by-UUID; legacy 64×32 auto-converted; imports token-correlated (no wrong-entry overwrite) + auto-equipped; `mc_import.rs`. *(PNG file import already shipped Phase 1c; native Stash sync deferred.)*
4. **Touch & polish.** Pinch-zoom / tap-paint, brush sizes, start-from-template, optional 2D precision panel (reuses `workshop_painter.rs`). Gamepad parked.

## 15. Reuse map

**Reused (exists):** `raycast::{pick_cell_in_cage, face_texel}` (hit→texel); `workshop_painter::paint_cell`, `override_registry::AuthoredFaces`, `EditBuffer::paint` (per-pixel RGBA); `workshop::symmetric_cells` (mirroring); `skin_uv::{base_faces, overlay_faces}` (forward UV, inverted here); `cosmetics::{CosmeticDescriptor, SkinSource, decode_skin_64, skin_key}` (data + decode + hash); `wasm_save::cosmetic_save_wasm/cosmetic_load_wasm` (web save); `entity_model::{player_model, build_skin_part_vertices, OVERLAY_INFLATE}` (mannequin render).

**New:** `Wardrobe`/`SkinEntry` + storage + migration; reverse UV map in `skin_uv.rs`; `skin_studio.rs` (mode state, tools, undo, layer); orbit/zoom edit camera; RGB/HSV colour-picker UI; native cosmetic persistence; Minecraft PNG export (web canvas + native rfd); Minecraft-username import (native reqwest + web `/mc-skin` proxy route); "Your look" wardrobe grid UI.

## 16. Decisions log

- **Home/model:** 3D paint-on-mannequin in the Workshop (reuses inflate→paint→pin) — chosen over a 2D canvas in "Your look" and over a hybrid.
- **Fidelity:** per-pixel brush + fill tool (real editor, not a recolour).
- **Edit buffer:** edit the 64×64 directly (buffer = skin) — chosen over per-face buffers + compositing.
- **Platforms:** web + native, one egui painter, two save backends.
- **Save:** overwrite the opened entry; duplicate to fork.
- **Minecraft path:** download/refresh only (one-way); store the resolved **UUID + handle** and refresh **by UUID**; web via our **own** game-server proxy talking straight to Mojang (not Crafatar / a third party); never a runtime dependency; no data retention.
- **Stash:** native-only, opt-in; upload (one) or sync (whole wardrobe).
- **Phasing:** base **and** outer layer both ship in Phase 1 (outer is the headline ask, a small delta on the base machinery); Minecraft PNG export pulled forward into Phase 2.

### Resolved during the 2026-06-30 Workshop-painter build

- **Painter delivery:** the painter is the **Workshop blow-up flow** (Bellows → charge/lock → crosshair dye-paint → P pin). The standalone Skin Studio orbit panel + offscreen `render_skin_preview` are **retired**. "Your look → Edit / New" teleports into the Workshop and auto-inflates the mannequin on that entry. The orbit-panel fallback (§6.6) is N/A.
- **Pin target:** **overwrites the opened / worn entry** (`skin_wardrobe::update_rgba(editing_id, …)`). The wardrobe `editing_id` is the equipped entry when you blow up the standing mannequin, or the chosen entry when you arrived via "Your look → Edit". Duplicate is the fork mechanism (§7, unchanged).
- **Live preview:** paint to **skin layer 0** (the worn layer) each stroke — free preview with no extra buffer. On cancel, `apply_local_cosmetic` restores layer 0 from the equipped wardrobe entry (no flicker in practice because the collapse animation plays first).
- **Carve / microblocks for skins:** **N/A.** The avatar painter is **Reskin only** — `WorkshopMode::Reshape`, `EditBuffer`, and `pick_cell_in_cage` are never reached on the `Avatar` target path. The Avatar `WorkshopProject` is `#[serde(skip)]`-transient and is never serialized in a `WorldSave`.
- **Base/Outer in-world toggle: V** (repurposed from Paint↔Sculpt — Sculpt is meaningless on an avatar; `WorkshopMode::Reshape` is never reached). No new keybind.
- **Eyedropper (G): override `picked_color`** — sets `SkinPaintSession.picked_color`, which supersedes the held dye for subsequent strokes. Cleared automatically when the held hotbar slot changes.
- **Left/right mirror (M): `skin_uv::mirror_hit`** — swaps arm_l↔arm_r and leg_l↔leg_r, swaps +X/−X faces, flips `frac_u`. In v1 (shipped).
- **Eraser gesture: empty-hand left-click** — Outer layer → alpha 0 (`skin_paint::erase`); Base layer → restore default-skin pixel (`skin_paint::restore_from` + `texture_gen::default_skin_rgba`).
- **Avatar model space:** feet at y=0, front on −Z, body rotated by `phi + π/2` about Y. `workshop::world_ray_to_avatar_model` inverts exactly this. `WORKSHOP_MANNEQUIN_YAW = π/2` so the front (−Z face) points toward spawn (+Z).
- **Per-stroke undo:** snapshot pushed on each rising-edge of left-click (stroke start), capped at 24 entries in `SkinPaintSession.undo`.

### Open items to resolve during planning

- Legacy 64×32 skin handling: convert vs friendly-reject (§8.4).
- Exact Mojang endpoint URLs (re-confirm; §8.4).
- Wardrobe size cap / localStorage budget warning threshold. **Resolved (Phase 1c):** none enforced in v1.
- Thumbnail rendering approach for the wardrobe grid (rendered avatar front vs 2D texture swatch). **Resolved (Phase 1c):** per-entry cached 3D mini-avatars rendered inline in the grid.
- Web persistence scope. **Resolved (Phase 1c):** localStorage-only (no Stash); Stash sync deferred to Phase 3.

---

## 17. Manual playtest checklist (Workshop painter, native release)

Run after the 2026-06-30 build to validate the in-Workshop painter end-to-end.

- [ ] Enter the Workshop → a mannequin wearing your equipped skin stands a few blocks ahead of spawn.
- [ ] Equip the Bellows; aim at the mannequin → a blue wire highlight appears around it; aim at a placed block → green/red block cage appears instead (both still work; nearest hit wins).
- [ ] Hold right-click on the mannequin → it swells ×1→×4 (charge animation); release early → collapses back to ×1 and vanishes.
- [ ] Hold right-click to full charge → mannequin locks at ×4. Sneak + right-click → collapses.
- [ ] With mannequin locked: hold a dye in your hotbar + left-click on a body part → the exact pixel paints, live, on the big mannequin AND your worn body in third-person (F5).
- [ ] **G** (eyedropper) while looking at a painted pixel → cross-hair samples that colour; painting elsewhere applies the sampled colour (hotbar item unchanged, override cleared when you switch slots).
- [ ] **M** → mirror on; paint one arm → the symmetric pixel on the other arm also paints.
- [ ] **The 2026-07-31 hole-punch (regression):** **V** to Outer → **G** on a bare (unpainted) clothes pixel → **V** back to Base → paint the back of an arm. The arm must take the colour **solid**, never go see-through. Orbit behind the mannequin to check the arm backs specifically.
- [ ] **The repair:** open a skin that already has a see-through patch via "Your look → Edit" → the patch is solid again on arrival (healed on load); **P** to pin, and it stays solid after relaunch.
- [ ] **V** → toggles Base / Outer; painting on Outer applies colour to the overlay rects; empty-hand click on Outer pixel → that pixel becomes transparent (base shows through); empty-hand click on Base pixel → restores the default skin colour.
- [ ] **P** → mannequin deflates, you are now wearing the painted skin in third-person; quit + relaunch → skin persists. "Your look" → the equipped entry's thumbnail reflects the paint.
- [ ] Blow up mannequin → paint → sneak-collapse (no P) → worn skin reverts to what it was before (no change saved).
- [ ] "Your look → Edit" on a non-equipped entry → teleports into the Workshop with the mannequin blown up and locked, loaded with that entry's skin → P saves back to that entry, NOT the equipped one.
- [ ] "Your look → New (blank)" → Workshop, blown up on a new empty entry.
- [ ] Per-entry **Export ⬇** still downloads a valid Minecraft PNG (open in GIMP / Minecraft launcher to verify).
- [ ] "Bring in a Minecraft skin" import dialog still works (enter a Minecraft username → skin imports).
- [ ] Wear / rename / duplicate / delete wardrobe entries unchanged.
- [ ] `./check.sh` exits 0 (clippy + build + 226+ tests + trunk build + bundle under 5 MiB brotli).
