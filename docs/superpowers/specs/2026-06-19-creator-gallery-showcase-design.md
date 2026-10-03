# Creator Gallery & Showcase Mode — Design

**Date:** 2026-06-19
**Status:** Phase 1 **locked** for implementation. Phases 2–4 framed (coherent, de-risked, not yet specced in detail). **Gaps & opportunities pass 2026-06-19** — resolved decisions in §13, remaining owner questions in §12, reconcile-before-build note in §14.
**Origin:** Grew out of the dedicated-server (`:8443`) web client. Opening that URL drops a visitor straight into the server world as a **fresh anonymous guest**, origin-isolated from any real login (verified: the page mints a throwaway key per load and loads none of the sign-in stack). That guest-auto-join is the wrong *default* for "me joining as myself" — but it's exactly right as a **zero-install, sandboxed, kiosk/booth experience**. This spec turns that into a deliberate, configurable showcase.

---

## 1. Summary

A configurable **showcase experience** that turns a dedicated AxeNStax server into a curated, walkable gallery:

> an artist builds a space → places **exhibits** (2D art on walls, or billboard "objects" on pedestals) → visitors walk it → optionally **collect** pieces into a per-visitor **basket** → **exit** through a configured screen.

One framework serves three purposes **by configuration** — a *taster/funnel*, an *inspiration board*, or a *gift shop* — and supports both **public** (open, Beacon-distributed) and **private / pay-gated** galleries (Stash-stored, consigned to the server, gated at the door).

It is deliberately **distinct from the "join as yourself" paths** (native LAN join via `ws://host:6767`; web from a real domain). This is the anonymous-guest, sandboxed-identity kiosk lane: galleries, event booths, "scan-this-QR" demos.

## 2. Goals

- **One configurable framework, not three features.** The skeleton is fixed; only two knobs vary (see §4).
- **The artist curates the space in-world** — builds the room with normal blocks, then **places / moves / resizes / re-orients** exhibits where they want. (Today's gallery can't do this — see §6.)
- **Both exhibit presentations**, both 2D for now: wall-hung art, and standing billboard "objects".
- **Useful immediately, leverageable later.** Phase 1 alone yields a curated, walkable gallery. Every later phase bolts onto the *same* exhibit / payload / exit seams — nothing gets rebuilt, it gets filled in.

## 3. Non-goals (explicit deferrals)

- **True 3D object exhibits.** Objects are represented as transparent **billboards** (a 2D plane that always faces the viewer). No 3D model pipeline. *(Avenue kept open: audio, text-panel, and voxel-build-adopt exhibits are **staged future presentations on the same generic primitive** — see §13 #5/#6 — not foreclosed.)*
- **A dedicated "plinth" block** or an "apply-plinth-to-any-block (keep its skin)" transform. Pedestals are simply built from normal blocks; an exhibit stands on top.
- **Front/back-distinct or double-sided art.** Wall art is single-sided (back = wall); standing art is a billboard (no back exists). Distinct-back and single-sided-standing become per-exhibit settings later.
- **Self-service upload, Beacon/Stash distribution, private/consignment, commerce checkout** — Phases 3–4.

## 4. Architecture overview

**Common skeleton (fixed across all configs):**

```
curated world  →  clickable exhibits  →  per-visitor basket  →  configured exit
```

**Two configuration knobs:**

1. **Exhibit payload** — what a piece carries/collects. Starts as `label` (+ image); grows to `link`, `sku`, `price`. Defined as fields from day one; later phases consume the extra fields.
2. **Exit action** — what the basket leads to: a *CTA* (get-the-game / sign up), an *inspiration board* (share / recreate), or a *checkout* (gift shop).

**Two orthogonal layers added *behind* the exhibit primitive later** (they do not change how an exhibit is placed, sized, or rendered):

- **Image source:** file-drop (Phase 1) → Beacon publish/adopt (Phase 3) → consigned private Stash (Phase 4).
- **Access gate:** open (Phase 1/2) → whitelisted npubs / pay-to-enter (Phase 4).

This orthogonality is the core architectural bet: **the exhibit primitive only needs "an image to display."** Provenance and gating sit behind it.

## 5. The Exhibit primitive (the spine)

An **Exhibit** is a placed, sized, payload-carrying 2D image surface in one of two presentations:

| | Geometry | Faces | Sides (v1) | Image | Used for |
|---|---|---|---|---|---|
| **Wall** | flat quad flush on a wall | fixed, into the room | single-sided (back = wall) | JPEG/PNG, opaque | paintings, posters, canvases |
| **Standing** | **Y-axis billboard** on/above a pedestal | **always toward the viewer** | n/a — no "back" exists | **PNG with transparency** (cut-out) | objects/statues/sculptures faked as sprites |

**Geometry rules (both presentations):**
- A **free quad** (not bound to block faces or block dimensions) — reusing the existing `renderer::painting` pipeline (see §6).
- **Size** is artist-set; a large piece is a bigger quad and may visually span multiple blocks.
- **Orientation** is an artist-set **yaw** — there is *no* "diagonal mode"; diagonal is simply 45°. Wall art's yaw is constrained to the wall it mounts on.
- **Billboard** rotates around the **vertical axis only** (stays upright as the viewer circles it; never tilts to face a viewer looking down — that would look floaty). Transparency via the engine's existing alpha pass (same mechanism as painted faces / plant rendering / avatar-skin alpha).

**Metadata (payload) fields** — defined now, partially used now:
- `label: String` *(used: plaque)*
- `presentation: Wall | Standing`
- `size: (w, h)` and `yaw` *(used: rendering)*
- `image_ref` *(used: which image to display)*
- `link: Option<String>`, `sku: Option<String>`, `price: Option<…>` *(defined; consumed in Phases 2/4)*

**Forward-compat — generic content payload (locked 2026-06-19, see §13 #5).** The v1 field shape above (`image_ref` + `presentation`) is deliberately a *special case* of a **content-generic** exhibit: think of it as `content_type` (`image/png` today) + a `ref`. The same `Vec<Exhibit>` later gains content-type-driven presentations — an **audio** exhibit (a speaker the visitor stands near), a **text panel** (a readable plaque), and a **voxel-build-adopt** exhibit (a plinth whose "collect" *adopts the build* into the visitor's own game). v1 implements image-only; the data + render dispatch are shaped so those slot in **without** a rebuild. This does not change the Phase-1 plan (which implements `image_ref`).

**Curation actions (in-world):** place, move, resize, re-orient (yaw), set presentation, set image, edit metadata, delete.

> **Why its own primitive, not an overloaded Vendor block:** we reuse the economy-block *pattern* (owner + config payload + click-to-interact) but keep Exhibit a distinct type — "proper abstractions, not X pretending to be Y."

## 6. Why today's Gallery can't be curated

> **Update 2026-09-29:** the built-in Gallery described below has been removed
> from the engine. It now ships as an optional external `.axeworld` world pack
> built on this spec's Exhibit primitive (each painting and plaque is a Wall
> exhibit, its image inside the archive), loaded via normal world import.

`game/engine/src/gallery.rs` is a **pure, deterministic procedural generator**: it computes a maze layout and auto-places hi-res paintings on the walls, consumed by `world::generate_gallery_column` (wall blocks) + `renderer::painting` (a textured quad per `Placement`). It was built to *generate*, never to be *authored* — there is no notion of moving a piece or choosing where it goes.

The asset we keep and build on is the **`renderer::painting` pipeline**: it already mounts **hi-res JPEG/PNG art as flat textured quads** (deliberately *not* the chunky 16×16 block textures — "low-res voxel walls + crisp art side by side IS the aesthetic"), with **title/artist plaques**, and loads art as content (native `include_bytes!` / web `js_load_gallery_art`). Phase 1 turns *placement* from computed into **authored + persisted**, and adds the **billboard** variant + **artist-set size/yaw**.

## 7. Authoring model

**Author natively, serve via dedicated.** The artist builds the room and places exhibits in **native single-player** (full edit access), where placements persist in the world save. The authored world is then exported (`.axeworld`, via the existing `world_archive`) and served by the dedicated server. This sidesteps the current alpha limitation that **web-joiner edits aren't yet propagated** to the server — authoring never depends on the web edit path.

Exhibit **images** live in the world's own folder (e.g. `<world>/exhibits/*.png|jpg`), dropped there by the artist or onto the dedicated server's `axenstax-worlds` volume. The headless server serves them same-origin; clients fetch + render via the painting pipeline.

> **Archive caveat (verified against `world_archive::pack_world`):** the current archive packs only `world_meta.json` + `world.dat` + `chunks/` — it does **not** bundle arbitrary files. So exhibit **definitions** travel in `.axeworld` automatically (they live in `WorldSave` → `world.dat`), but exhibit **image bytes** do not yet. Extending `pack_world`/`unpack_world` to carry `exhibits/` images (exactly as they carry `chunks/`) is a small foundation follow-up — required for Phase 3 portability, **not** for v1 local authoring (where images sit on the serving host).

## 8. Phasing

| Phase | Scope | Delivers |
|---|---|---|
| **1 — Curatable Exhibits** *(locked)* | Exhibit primitive (wall + standing billboard), in-world place/move/resize/orient, metadata fields, persistence, rendering. **Images via file-drop.** | A curated, walkable gallery you fully control |
| **2 — Visitor kiosk loop** | Showcase/containment mode (exit → terminal screen, no lobby back-door), per-visitor **basket**, **click-to-collect**, one simple **exit action** (CTA/board) | The unattended interactive demo loop |
| **3 — Content pipeline (public)** | **Studio** sidecar UX (Docker service behind Caddy, like the Operator Console at `/admin`); pull/push **Stash**; publish/adopt via **Beacon** | Artists onboard their own art, no dev involvement |
| **4 — Private galleries + commerce** | Gate at join (**whitelist npubs / pay-to-enter**); private art in **Stash**, **consigned** to the server identity (signed, expiring, **revocable**); SKU/price → **checkout** | Gated private galleries + the literal "gift shop" exit |

## 9. Phase 1 — Curatable Exhibits (LOCKED)

**Outcome:** an artist builds a room, drops their images into the world's `exhibits/` folder, places wall + standing-billboard exhibits exactly where they want, sizes/orients them, sets a label, and walks the result. Persists in the world; serveable by the dedicated server.

**Components**
1. **Exhibit data model** — a serializable struct carrying `presentation`, `image_ref`, `size`, `yaw`, anchor position, and the metadata fields (§5). Lives in a new `exhibit` module (pure data + helpers, mirroring `gallery.rs`'s "pure core" style).
2. **Persistence** — exhibit *placements/definitions* stored as a `Vec<Exhibit>` on `WorldSave` (alongside the existing `carts`); these ride `world.dat`, so they travel in `.axeworld` automatically. Image *bytes* are separate (see §7) and are **not** packed by the current `world_archive`.
3. **Rendering** — extend the `renderer::painting` path to: (a) draw **authored** placements (not just procedural ones), (b) add the **Y-axis billboard** orientation, (c) honour **artist-set size**, (d) support **alpha PNG** for billboards.
4. **Image source (file-drop)** — load images from `<world>/exhibits/`; native reads from disk, the dedicated server serves them same-origin for web clients to fetch.
5. **Curation UX (in-world, native)** — place an exhibit, choose its image from the dropped set, set presentation/size/yaw/label, move/delete. (Exact affordance — a placement tool vs. reusing a Workshop-style interaction — resolved in the Phase 1 plan; follow existing economy-block/Workshop placement patterns.)

**Data flow**
`<world>/exhibits/*.png` (artist drops) → exhibit defs authored in-world + saved into `WorldSave` (→ `world.dat`) → dedicated server loads the world + serves the image files same-origin → client receives exhibit defs over the protocol + fetches images same-origin → `renderer::painting` draws wall quads / billboards. (Definitions also travel in `.axeworld`; image bytes now do too — `pack_world`/`unpack_world` carry `exhibits/<ref>` members since P2a.)

> **Web file-import render path (#127, fixed 2026-06-22).** A `.axeworld` opened on the **web** client renders its bundled exhibit images **without** a server route. The WASM load path keeps `unpack_world`'s images (`State.pending_exhibit_images`); `request_exhibit_art` decodes those in preference to the same-origin `/exhibits/<ref>` fetch, looking each one up with `world_archive::exhibit_bytes_for` (record ref sanitised via `save::sanitize_image_ref`, mirroring native). The server fetch stays as the fallback for **published** worlds whose blob omits the image. Before this, web import discarded the images and the loader only hit `/exhibits/<ref>` → 404 → blank exhibits, while native worked (it persists images to `worlds/<name>/exhibits/`). Native is unchanged. Regression: `world_archive::exhibit_bytes_for_matches_sanitised_ref`.

**Testing** — pure-function unit tests on the exhibit module (serialization round-trip; billboard yaw/size math; placement validity), in the established `#[cfg(test)]` style. Save/load round-trip integration test (exhibit **definitions** survive `.axeworld` pack/unpack via `world.dat`), alongside the existing `save_load` tests.

**Modules touched** — new `exhibit.rs`; `gallery.rs`/`renderer` painting path; `save.rs`/`world_archive.rs` (persist + pack); world/world-gen for placement; a curation entry point in the menu/HUD or a Workshop-adjacent tool.

**Phase 1 explicitly excludes** — containment/kiosk mode, basket, collect, exit screens (Phase 2); upload/Studio/Beacon (Phase 3); gating/consignment/commerce (Phase 4); 3D objects, plinth block, distinct-back art (non-goals).

## 10. Phases 2–4 (framed)

**Phase 2 — Visitor kiosk loop.** A **showcase mode** (likely a dedicated-server flag) that: intercepts exit so it can't fall back to `GameMode::Menu` ("exit out of game *and* lobby in one go" → a terminal screen); maintains a **per-visitor basket** (client-side per guest session — the world is shared, the basket is personal); makes exhibits **clickable-to-collect** (plinth/wall = block-or-quad raycast); and renders one configured **exit action** (start with the cheap CTA/board). Optional auto-loop-to-fresh-session for unattended booths.

**Phase 3 — Content pipeline (public).** A **Studio** web UX shipped as a Docker **sidecar** behind the same Caddy front (exactly the Operator Console pattern: a service at, e.g., `/studio`, sharing the `axenstax-worlds` volume — *not* baked into the engine image). Artists pull/push their own **Stash** and **publish to Beacon**; galleries **adopt** Beacon pieces, reusing the shipped Workshop→Beacon→adopt path.

**Phase 4 — Private galleries + commerce.**
- **Privacy is enforced at the door, not the storage.** Public art → Beacon (open). Private art → the artist's **Stash** (encrypted), with the *world* **gated at join** via the dedicated server's existing access control (`AXENSTAX_WHITELIST` / `REQUIRE_SIGNIN`, or the specced non-refundable **pay-to-enter** entrance). Admitted visitors receive art only through the gated game stream; it is never on a public Beacon to be scraped.
- **Consignment:** the artist grants the server's **Heartwood-signed runtime identity** access to specific pieces (NIP-44 re-wrap the content key to the server's npub). The server decrypts **server-side** and streams to admitted players.
- **Revocable:** the consignment is a **signed, time-bounded grant** (expiry + renewal — the same pattern the server-identity delegation already uses). Revoke by publishing a revocation or not renewing; the server re-checks and drops the art. **Honest caveat:** revocation stops *future* display — it cannot un-see what a visitor already had on screen (true of any displayed media).
- **Commerce exit:** `sku`/`price` payload → basket → checkout via the merch/claim store. Payment routes **directly to the artist** (their wallet/LNURL) — **no platform split** (§13 #12); the claim store handles fulfilment / claim-code, not a funds cut.

These reuse shipped primitives — **server identity** (grantee), **access policy** (door), **Stash + NIP-44** (encrypted storage + consign), **signed/expiring grants** (revocable consignment), **Beacon** (public distribution), the **Operator Console sidecar pattern** (Studio hosting).

## 11. Cross-cutting seams (the "leverageable later" guarantees)

- **Payload is extensible** — `link/sku/price` exist from Phase 1, consumed later.
- **Content is type-generic** — `image/png` first, but `content_type` + `ref` lets audio / text / voxel-build-adopt presentations ride the same primitive (§13 #5/#6).
- **Exit action is a slot** — CTA → board → checkout all plug into one interface (Phase 2+); the slot also owns the **no-account take-away artifact** (share link/QR, or claim code) so a guest leaves with their basket (§13 #2).
- **Image source sits behind the primitive** — file-drop → Beacon → consigned Stash, no change to placement/rendering.
- **Access gate sits at the join boundary** — open → whitelist → pay, independent of exhibit code.
- **The containment/flow/exit seam is experience-generic** — kept general enough to later host a demo arena / tournament / classroom / shop, not gallery-specific (§13 #9).
- **Cross-game-liftable** — showcase mode, consignment, and the Studio are kept general (Forgesworn boundary), reusable by other Decented games (§13 #10).

## 12. Open questions

**Resolve during the build (implementer/plan-level):**
- Curation affordance: a bespoke placement tool vs. reusing a Workshop-style interaction (resolve in the Phase 1 plan; 1c recommends command-driven `/exhibit …`).
- Billboard transparency: alpha-test (hard cut-out edges) vs. alpha-blend (soft) (1b resolves to alpha-test).
- Phase 2: showcase mode as an env flag (e.g. `AXENSTAX_SHOWCASE=1`) + exit-loop behaviour (auto-loop vs dead-end vs configurable).
- Phase 3/4: per-gallery default of Beacon (public) vs Stash (private); Studio as its own sidecar vs an extension of the Operator Console.

**Resolved 2026-06-19 (owner pass) — now locked in §13:**
- **Q-A → §13 #1:** per-card / share-link discovery first; a central directory is a deferred, optional later layer.
- **Q-B → §13 #12:** **no platform revenue split** — 100% direct to the artist; only the artist's payment-rail fees apply; hosting may be a separate charge (never a per-sale split).
- **Q-C → §13 #9:** showcase stays a standalone `GameState` flag now; the general seam keeps a future "Experience" unification open.
- **Q-D → §13 #13:** default phase order holds; build-adopt + discovery stay in their later slots, revisited at build time.

*Residual (a separate business topic, not a gallery blocker):* the **hosting-fee model** — whether/how we charge operators to host — is out of scope for this spec.

## 13. Resolved decisions (gaps & opportunities pass, 2026-06-19)

Each is **long-run by design, staged-friendly, and forecloses nothing.** Phase tags say where the *work* lands; the *decision* is locked now.

1. **Discovery = per-card / share-link first (Q-A resolved 2026-06-19).** A creator shares their gallery's address (a link / QR; the **Server Card**, kind-30422 / server-creator-UX spec A, resolves npub→address), **opt-in, off by default** (privacy-first). A **central directory/aggregator of galleries is a deferred, optional later layer** — avenue kept open, but no curation/moderation burden taken on now. Reuses shipped infra. → *announce: Phase 3 / whenever Server Card lands.*
2. **Basket leaves via a no-account artifact.** The per-visitor basket is **ephemeral (session-scoped)**; the **exit action** carries it out without the guest having an account — a **shareable link/QR** (CTA / inspiration board) or a **claim code** (commerce). → *Phase 2 (link/QR), Phase 4 (claim code).*
3. **Commerce = claim-service code, non-custodial.** Gift-shop checkout mints a **claim code** via the existing claim service (`tools/sites/claim`); no guest account; non-custodial per ADR-004. → *Phase 4.*
4. **Safe default = Adventure (read-only) + locked building + operator moderation.** Showcase mode defaults to the shipped **Adventure `PlayMode`** (read-only to edits), building locked, operator kick via the console; an operator may open it up. Safe-by-default for anonymous public worlds. → *Phase 2.*
5. **Generic content payload (the key "don't close avenues" lock).** The Exhibit primitive is **content-type-generic** (`content_type` + `ref`), not image-only. **2D image is the first presentation;** audio (speaker), text panel, and voxel-build-adopt are future presentations on the *same* primitive + render-dispatch. → *Phase 1 shapes it generic; image-first; others later.* (Does not change the locked 1a plan.)
6. **Voxel-build-adopt presentation (high-synergy future).** A plinth whose exhibit references a shareable **Plan/build**; "collecting" it = **adopting it into the visitor's own game**, reusing the shipped **Workshop→Beacon→adopt** path. → *future phase (reuses Beacon-adopt; build-order Q-D).*
7. **Public galleries run on a real domain.** Use the existing **`AXENSTAX_DOMAIN` / `Caddyfile.domain`** Let's-Encrypt path to kill the cert warning; **self-signed is LAN/private/dev only.** A QR/entry-link generator is a Studio/operator feature. → *Phase 3 / operator docs.*
8. **Demo-kiosk slice = a near-term standalone win.** A minimal kiosk — **containment + safe Adventure default + a simple exit CTA, on a pre-built world, with NO exhibits** — is an independently-shippable subset of Phase 2, valuable for events/conferences now, decoupled from the curator pipeline (Phase 1). → *Phase 2a; may precede the full gallery.*
9. **The experience seam is kept general (Q-C resolved 2026-06-19: separate now, unify later if needed).** Containment + curated-flow + exit is a general **"hosted experience"** seam, not gallery-specific — so it can later host a demo arena / tournament / classroom / shop. Showcase is implemented as its **own `GameState` flag** (Phase 2), **not** folded into the objective-driven Scenario system (a Scenario's end-card returns to the lobby — the exact back-door showcase must remove). Keeping this seam general preserves a future merge into one unified "Experience" abstraction **without forcing it now**. → *design constraint, Phase 2.*
10. **Cross-game-liftable.** Showcase mode, consignment, and the Studio sidecar are kept **general (Forgesworn boundary)** so they lift to other Decented games — not AxeNStax-specific repackaging. → *design constraint.*
11. **Positioning.** This is the **concrete first instance of the player-driven-economies vision** (`docs/vision/economies-long-run.md`) — the creator-display / spectator economy made real, and the tangible face of "creators run monetised servers." → *framing.*
12. **Revenue: NO platform split — 100% direct to the artist (Q-B, owner decision 2026-06-19).** A sale routes payment **directly to the artist** (their own wallet / LNURL); the platform/service-provider takes **no per-sale cut, ever**. The only deductions are **fees intrinsic to the artist's chosen payment rail** (e.g. Lightning routing / phoenixd), borne by the artist — never a platform split. **Hosting** *may* be charged separately (a fee for running the box), which is a **distinct arrangement, never structured as a per-sale split**. Fully consistent with the non-custodial, never-touch-funds posture (ADR-004 / "not a money transmitter"). → *Phase 4: commerce routes direct-to-artist; no settlement-split code needed.*
13. **Build-order: default sequence holds; revisit at build time (Q-D resolved 2026-06-19).** Build **Phase 1 (exhibits) + the demo-kiosk slice (#8) first** (independently valuable, de-risks the rest), then 2 → 3 → 4 by demand. **Voxel-build-adopt (#6) and discovery (#1) stay in their later default slots** unless demand says otherwise — decided when we reach them, not committed upfront. Staged and non-closing.

## 14. Reconcile before build (mandatory step 0)

This spec + all six plans were grounded against the code as of **2026-06-19**; the engine is being changed by other sessions. **Before executing any plan, re-verify every `path:line` reference and interface, and amend the plan.** Known drift already:

- **`WorldSave` append-order has moved.** The concurrent "Solo Buildout Wave 2" appended new last fields (`signs`, `item_frames`, `locked_slots`, `hostile_acts`). The 1a plan's "append `exhibits` **last**, read via `read_tail`" rule still holds — but the *neighbour* field changed (no longer `power_devices`/`graves`). Re-`grep` `pub struct WorldSave` + every `WorldSave { … }` construction site before adding the field.
- **The block-shape foundation** (top of the foundations queue) touches block state / collision / mesh — the same neighbourhood as the `renderer::painting` path (1b) and block interaction (1c/2). Re-confirm those integration points haven't shifted.
- **`save.rs`, `renderer.rs`, `game_loop.rs`, `main.rs`** are high-churn; treat every line reference in 1a–1c + Phase 2 as "verify, don't trust."
