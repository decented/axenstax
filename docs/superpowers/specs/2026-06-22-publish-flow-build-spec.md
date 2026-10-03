# Publish flow — BUILD SPEC

**Date:** 2026-06-22
**Status:** BUILD-READY (cleared for engine work; not yet built).
**Scope:** the **publish-from-game** loop — *initial publish from the console, updates from the game* —
the in-game **Publish** action + **badge**, the console **"add a world"** + **export** endpoints, the
**content handoff** to the dedicated server, and the **gallery image transfer** gap.
**Parent design:** `2026-06-22-multi-world-hub-and-portals-design.md` §1 (Build elsewhere, publish
from the game; Stash≠Publish; gallery = capability).
**Out of scope (separate specs / later):** portals + the multi-world hub, Aether, named biomes
(beyond normal/flat), live collaborative building on the server.

---

## 0. The loop (what we're building)

1. **Initial publish — console.** Operator opens `/admin` → **Add a world**: name, **kind** (normal /
   gallery), **terrain** (normal seed / flat+ground), **access**. The server **creates + serves** that
   world (terrain only). *(This is the existing wizard, generalised to "add a world".)*
2. **Pull to edit — game.** In the normal client, the operator's server worlds appear with an **Edit**
   action that downloads the served world as `.axeworld` into their local worlds + Stash.
3. **Edit → Stash → Publish — game.** Edit locally (works on web *and* native — it's a local world,
   so the web-edit-propagation blocker doesn't apply). **Stash = save this version** (private).
   **Publish = push the snapshot to the server** when ready. Badge shows live-vs-editing state.

Key property: the operator **never edits the live server world directly** — they edit a local copy and
publish snapshots. Draft → publish, like a website.

---

## 1. Architecture (transport + reload)

```
  GAME (web/native)                 CONSOLE  (FastAPI, /admin, operator-gated)         ENGINE (--server)
  ─ pack_world() → .axeworld    ─POST /admin/api/world/<id>/publish──▶  verify sig +   ─ entrypoint sees
  ─ + exhibit images               (.axeworld + images + operator-       npub==operator   .identity/restart
  ─ sign authz event  ───────────▶  signed authz event)               → write world to → reboots serving
                                  ─GET  /admin/api/world/<id>/export ◀── volume slot +    the new world
  ─ import .axeworld (pull)         (stream .axeworld)                  drop sentinels
                                  ─POST /admin/api/world/create (initial)
```

- **HTTP host = the console** (Python). The engine stays HTTP-free (the whole reason the console
  sidecar exists). The console writes to the **shared `/worlds` volume** and drops the
  **`.identity/restart`** (and `reset-world`) sentinels the entrypoint already honours; the engine
  reloads. Reuses: `tools/dedicated-server/entrypoint.sh` (restart/reset-world), `save::load_world`.
- **Transport = multipart HTTP** (large binary; reuse the `studio_upload` pattern in
  `console/app.py`).
- **Auth = an operator-signed authorization event** carried *in the request* (not a browser cookie),
  verified server-side — see §3.

---

## 2. Data model

### 2.1 `WorldMeta.published_to` (engine, `save.rs`) — append-only, `#[serde(default)]`
```rust
#[serde(default)]
pub published_to: Option<PublishRecord>,

pub struct PublishRecord {
    pub server_npub: String,     // operator npub of the target server (the join/own key)
    pub server_addr: String,     // wss://… or host:port (display + re-publish target)
    pub world_id: String,        // the served-world slot id on that server
    pub last_published_hash: String,  // SHA256 of the .axeworld bytes last published
    pub last_published_unix: u64,
}
```
Drives the **badge**: *not published* (None) / *up to date* (hash == current pack hash) / *unpublished
changes* (hash != current). Append-only ⇒ zero save-format-break risk (recon confirmed tolerant decode
+ `#[serde(default)]`).

### 2.2 Served-world slot + manifest (console, on the volume)
Each served world is a dir under `/worlds/<world_id>/` plus an entry in a console-managed
`/worlds/.identity/worlds.json` (the manifest the multi-world spec defines):
`{ id, title, kind: "normal"|"gallery", access, content_owner_npub, world_type, ground, created_unix }`.
(MVP can serve a single world via the existing `AXENSTAX_WORLD`; the manifest generalises to many in
the multi-world build.)

### 2.3 `.axeworld` **extended to carry exhibit images** (engine, `world_archive.rs`) — **the gallery fix**
Today `pack_world`/`unpack_world` bundle `world_meta.json` + `world.dat` (+ exhibits *placements* in
`WorldSave.exhibits`) + chunks, but **NOT the exhibit image files** (`worlds/<name>/exhibits/*`). For a
gallery publish the images must travel. **Add an `exhibits/<ref>` member to the tar** on pack, and
restore it on unpack. Bound total size + keep the existing path-traversal + decompression-bomb guards.

---

## 3. Auth — operator-signed publish authorization

The game is not the console browser, so it carries its own proof instead of a cookie:

- The game builds an **authorization event** (Nostr event, operator-signed) binding:
  `{ world_id, server_npub, archive_sha256, created_at, nonce }`.
- Web signs via the retained signer: `window.__axenstax_get_signer().signEvent(...)`
  (`wasm_auth::has_js_signer`). Native signs via `signet/native_signer.rs`.
- The console `/api/world/<id>/publish` **verifies** it with the **same secp256k1 it already uses in
  `auth.py`**: signature valid → `pubkey == server operator npub` (`identity.operator_pubkeys_hex()`)
  → fresh (±300 s, mirroring `server_identity/admin.rs`) → `archive_sha256` matches the uploaded bytes
  (anti-replay/anti-swap). Reuses the admin-command verify *philosophy*; implemented in Python.
- **No console session cookie needed for publish.** (The create/export endpoints can use either the
  cookie — operator already in the browser — or the same signed-event scheme; pick cookie for the
  browser-driven create, signed-event for the game-driven publish.)

---

## 4. Endpoints (console — `tools/sites/console/app.py`, operator-gated)

| Endpoint | Auth | Does |
|---|---|---|
| `POST /api/world/create` | cookie + `_require_cap("settings")` | **Initial publish.** Writes a new `worlds/<id>/` meta (kind/world_type/ground/access) + manifest entry + `content_owner_npub`; drops `reset-world` + `server.env` so the engine **generates** it. (Generalises the wizard.) |
| `GET /api/world/<id>/export` | cookie or signed-event | **Pull to edit.** Streams the served world packed as `.axeworld` (engine-side pack at rest, or pack from the on-disk dir). |
| `POST /api/world/<id>/publish` | **operator-signed authz event** (§3) | **Update publish.** Multipart: `.axeworld` (+ images already inside it after §2.3). Verify → unpack into `worlds/<id>/` → update manifest + `published` state → drop `restart` sentinel → engine reloads. |
| `GET /api/worlds` | cookie | List served worlds + their state (for the console arrangement screen). |

Model the upload on the existing `studio_upload` handler (`app.py`); reuse `_require_fetch` (CSRF),
`identity.*` for the volume paths, and `auth.py`'s verifier for the signed event.

---

## 5. Engine changes (focused)

1. **`world_archive.rs`** — include `exhibits/*` images in pack/unpack (§2.3). *Required for gallery
   publish.* Unit-test round-trip incl. an image. (Touches `save::exhibit_image_path`.)
2. **`save.rs`** — add `WorldMeta.published_to: Option<PublishRecord>` (append-only) + a helper to
   compute the current pack hash for the badge.
3. **Served-world gallery flag** — when the slot's `kind == "gallery"`, the engine serves it with
   showcase/read-only-visitor behaviour (reuse the existing `showcase` path; the manifest/`server.env`
   carries it). No new toggle.
4. **Native HTTP (Phase 4 only)** — add `reqwest` (multipart) to the native target deps for native
   publish; web uses `fetch`. (Native currently has *no* HTTP client — recon.)
5. **No world-gen changes for MVP** — biome picker uses existing `world_type`("normal"/"flat") +
   `ground`. **Named biomes (waterfront/forest) are deferred** (needs terrain-gen work; see §9).

The engine stays HTTP-free; all publish HTTP is the console.

## 6. Client (game) changes

1. **Publish action** on a world (lobby card + in-world pause menu): `pack_world()` → compute hash →
   sign authz (§3) → `POST …/publish` (web `fetch`; native `reqwest` in P4). Show progress + result.
2. **Pull-to-edit**: an **Edit** action on a server world → `GET …/export` → `unpack_world` into a
   local world + Stash it.
3. **Badge** from `WorldMeta.published_to`: *not published* / *published to <server> · up to date* /
   *· unpublished changes* (hash compare). Lobby card + pause menu.
4. **Publish targets** = the operator's servers. MVP: operator picks the server (address) the first
   time; thereafter `published_to` remembers it. (`my_servers.rs` already stores entries; *operator
   detection* — confirming signed-in npub == server operator — can be MVP'd by "attempt publish; server
   authorizes or 403s", with a nicer attestation-fetch later.)
5. **Stash reuse**: `pack_world()` bytes feed *both* the existing `cloud_save_wasm` (Stash) and the new
   publish upload — same bytes, two destinations, independent actions (Stash≠Publish).

---

## 7. End-to-end flows

**A — Initial create (console):** operator → `/admin` → Add a world (name/kind/terrain/access) →
`POST /api/world/create` → meta+manifest written, `reset-world`+`server.env` dropped → engine generates
+ serves it. Manifest marks `content_owner = operator`.

**B — Pull to edit (game):** operator's client lists their server worlds → **Edit** → `GET …/export`
→ `unpack_world` → local editable world (+ Stash). Sets `published_to` = that server/world/hash.

**C — Edit → Stash → Publish (game):** edit locally; **Stash** saves versions (private, no server).
When ready, **Publish** → pack (+images) → sign → `POST …/publish` → verify → unpack into slot →
`restart` sentinel → engine reloads → badge → *up to date*. Re-publish overwrites (prior world archived
by the existing `reset-world`/backup behaviour).

---

## 8. Build order (each milestone independently verifiable)

- **P0 — badge plumbing (engine).** `WorldMeta.published_to` + pack-hash helper + badge in lobby.
  *Test:* save round-trips with/without the field; badge states render. Low risk, no transport yet.
- **P1 — console "Add a world" / initial publish.** `POST /api/world/create` + a console screen;
  reuses wizard + reset-world. *Test:* create → engine serves a fresh world of the chosen kind/terrain.
  Mostly Python.
- **P2 — `.axeworld` carries images (engine) + `GET …/export` + pull-to-edit (game).** *Test:* pack/
  unpack round-trip incl. an exhibit image; export from server → import locally shows the art.
- **P3 — the core: `POST …/publish` + operator-signed auth (console verify) + game Publish (web) +
  reload.** *Test:* publish from web → server reloads serving it; tampered/foreign-npub/stale authz →
  403; badge → up-to-date.
- **P4 — native publish** (add `reqwest`). *Test:* publish from native.
- **P5 — gallery "My Art" library + placement** (gated to gallery worlds). *May be its own slice* —
  depends on the assets/inventory work; without it you can still publish pre-placed exhibits.

`./check.sh` gates every engine-touching milestone (clippy/build/tests/trunk/bundle-size). Add unit
tests in `world_archive.rs`, `save.rs`; integration in `test_integration/`; a console `test_*.py` for
the verify + endpoint logic.

## 9. Risks & open questions

- **Named biomes deferred.** "Waterfront gallery" needs terrain-gen work; MVP picker = normal/flat +
  ground. Confirm that's acceptable for v1 (recommend yes; richer biomes as a follow-on world-gen spec).
- **Archive size with images.** Galleries can carry many MB of images — bound the archive, stream the
  upload, and set a sane max (mirror the 12 MiB/image studio limit × a cap).
- **Operator detection UX.** MVP "attempt → 403" works; a relay attestation fetch to pre-mark "you
  operate this" is nicer (ties to the My Servers / Server Card work).
- **Auth replay/swap.** The signed authz binds the archive hash + a nonce + freshness — verify all
  three. Reuse `auth.py` crypto.
- **Single vs multi world.** MVP can serve one world (existing `AXENSTAX_WORLD`); the manifest +
  per-world slots generalise to many when the multi-world hub is built (separate spec).
- **Content-safety** for published gallery images is **out of scope here** (flagged in the multi-world
  spec; parked by owner) — but the publish endpoint is the natural future home for the gate.

## 10. Explicitly deferred
Portals / multi-world hub / menu-lobby (separate spec); Aether (separate spec); named biomes;
live collaborative building; content-safety scanning; the gallery art-library UI if P5 is split out.
