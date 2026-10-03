# Local World Backup — export/import a world to a file (PWA)

**Date:** 2026-05-27
**Status:** APPROVED — ready to build. PWA/WASM first.
**Trigger:** Offline worlds live only in one browser's IndexedDB — a cleared cache or a new device loses them. This is the **no-infrastructure backup rung**: save a world to a file, load it back. Precursor to the online cloud-sync layer ([`../../foundations/2026-05-26-cloud-save-blossom.md`](../../foundations/2026-05-26-cloud-save-blossom.md), Spec 31), which reuses the same packed-world blob — but needs no server and no upstream Signet capability, so it's buildable now.

## Goal

A player can **Back up a world to a file** on their device and **Restore from a file** later (same device, another browser, or another machine). Protects builds against a cleared cache without any server.

## Architecture (follows existing patterns)

Worlds are listed/managed in the in-game world menu (`menu.rs`, egui), stored as a tar+gzip blob in IndexedDB via the `window.axenstax_*` bridges, with `pack_world`/`unpack_world` in `wasm_save.rs`. Two new surfaces hang off that:

### Export — per-world "Back up to file" button
The world's packed blob already sits in IndexedDB, so export is **mostly JS**:
- `world_store.js`: new `window.axenstax_export_world(pubkey, name)` — read the IDB record's blob, wrap in a `Blob`, trigger a browser download named `<name>.axeworld`.
- `menu.rs`: a per-world button calls `wasm_save::export_world_wasm(pubkey, name)` (a thin `spawn_local` fire-and-forget wrapper over the JS bridge). WASM-only.

### Import — top-level "Restore from file" button
- `world_store.js`: new `window.axenstax_pick_world_file()` — create a hidden `<input type="file" accept=".axeworld">`, resolve with `{ name, bytes }` (ArrayBuffer → Uint8Array) once the user picks, or `null` if cancelled.
- `wasm_save.rs`: `import_world_wasm(pubkey)` — await the picker → `unpack_world(bytes)` to **validate** it's a real Axe'n'Stax world and read its `WorldMeta`/name → compute a non-colliding name → `js_save_world(pubkey, final_name, bytes, meta_json)`. Returns `Result<String, String>` (the imported world name, or a friendly error).
- `menu.rs`: a top-level button kicks off the async import using the **same async-poll slot pattern as `list_worlds_wasm`** (`menu.rs:205`); on success, refresh the world list + show a status line; on a bad file, show "That's not an Axe'n'Stax world."

### Name collision → import as a copy
Pure helper `dedupe_world_name(name, existing: &[String]) -> String`: if `name` is taken, return `"<name> (imported)"`, then `"<name> (imported 2)"`, etc. **Never overwrites.** Unit-tested.

## Decisions (approved 2026-05-27)
- **Extension** `.axeworld` (self-describing; contents are the existing tar+gzip pack).
- **Wording** "Back up to file" / "Restore from file" — backup framing, not "cloud" (that's the later online layer).
- **Collision** import-as-copy, never overwrite.
- **Scope** PWA/WASM first. Native already has filesystem worlds; wiring the same two buttons to native file dialogs is a deferred follow-on (hide/disable them on native for now).

## Files touched
- `tools/sites/game/static/world_store.js` — `axenstax_export_world`, `axenstax_pick_world_file`.
- `game/engine/src/wasm_save.rs` — two `extern` bridges + `export_world_wasm` / `import_world_wasm`; pure `dedupe_world_name`.
- `game/engine/src/menu.rs` — per-world Back-up button + top-level Restore button + async import poll + status surfacing (all `#[cfg(target_arch = "wasm32")]`).

## Verification
- **Unit-testable (engine):** `dedupe_world_name` collisions; `pack_world` → `unpack_world` round-trip integrity (likely already covered — extend if not).
- **Playtest boundary:** the actual browser download + file-picker UX is not solo-verifiable — Axolittle confirms (download a world, clear cache, restore it). Same boundary as the rest of the PWA work.
- `check.sh` green; engine + WASM build.

## Memory-rule check
- **`project_pwa_priority` / shared-infra:** browser-generic; only the world blob is AxeNStax-specific. ✅
- **`project_alpha_launch_posture`:** no server, no deploy infra — respects "don't pre-build deploy infra." ✅
- **`feedback_npub_only_display`:** no identity rendering here; worlds keyed by the signed-in pubkey internally. ✅
- Precursor to Spec 31 (same blob); when cloud sync lands, "back up to file" and "back up online" sit side by side. UK English.
