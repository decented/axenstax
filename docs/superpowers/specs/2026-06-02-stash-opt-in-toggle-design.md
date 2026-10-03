# Stash Opt-In Toggle — Layer 1 design

**Date:** 2026-06-02
**Status:** APPROVED (Staxolottle, 2026-06-02) — build now.
**Branch:** `feat/axenstax-consume-stash`
**See also:** the always-on Stash build it amends (`cloud.js`, `save.rs:936`,
`menu.rs` cloud merge); the broader save architecture
(`docs/architecture/2026-06-01-persona-save-vault.md`); versioning/forking
(`docs/research/2026-04-04-world-versioning-forking.md`).

---

## Problem

Cloud save ("Stash") currently fires on **every** save whenever a NIP-44-capable
signer is present (`save.rs:936` gates only on `cloud_available()`). The original
design called for a **per-world opt-in, default OFF**
(`2026-06-01-cloud-save-full-loop-design.md:66`,
`persona-save-vault.md:81`) so random little tests stay local and only worlds you
choose follow you across devices. The toggle was specced but never built.

## Scope — Layer 1 only

This spec is the **opt-in policy + its UI surfaces + honest status**. It does NOT
cover: Stash lifecycle beyond toggle (deletion UX), multi-device conflict
resolution beyond last-write-wins, the Nostr provenance/version chain, or
sharing/forking. Those are named follow-on specs (Layers 2–3).

## Decisions (owner-approved)

- **Default OFF**, per-world.
- **Toggle off keeps the cloud copy** — it stops *pushing*; the world goes "out
  of sync". It does **not** delete from Stash. (Deletion is a separate explicit
  action, a future Layer-2 item.)
- **Multi-device = last-write-wins** by save recency — fine for now. No live sync.
- **Retrieval stays list + on-demand** (already built): cloud-only worlds appear
  as a list, download + decrypt only when opened. Unchanged.

## Data model

Add to `WorldMeta` (`save.rs`):

```rust
/// Whether this world pushes to the player's Stash (cloud) on save.
/// Per-world opt-in, default OFF — random/local worlds stay on-device;
/// only worlds the player flips on follow them across machines.
#[serde(default)]
pub cloud_save: bool,
```

`#[serde(default)]` ⇒ every legacy save (and the JS world-list payload) reads
back `false`. The synthetic cloud-only entry literal (`menu.rs` cloud merge) must
add `cloud_save: true` (a cloud-only world is, by definition, stashed).

## Gating (the actual behaviour change)

`save.rs` WASM save path, replace the gate:

```rust
// before: if crate::wasm_save::cloud_available() {
if meta.cloud_save && crate::wasm_save::cloud_available() {
```

Extract the predicate as a pure, unit-tested free function:

```rust
/// Stash push happens only when the world opted in AND a capable signer exists.
pub fn should_stash(world_opted_in: bool, signer_capable: bool) -> bool {
    world_opted_in && signer_capable
}
```

When `cloud_save` is false the JS bridge is never called, so no "Stashing…"
toast fires — silence is correct for a local-only save.

## UI surfaces

1. **Create New World dialog** (`MenuDialog::Create`): add a `cloud_save: bool`
   field (default `false`) and a "STASH" ON/OFF toggle mirroring the existing
   COMMANDS toggle. Threaded into the new world's meta on confirm.
   Label copy: **"Save to your Stash"**, helper "Off — this world stays on this
   computer. On — open it on any computer."

2. **Lobby world card** (selected action bar): a toggle button
   **"📦 Stash: On" / "Stash: Off"** for local worlds. Flipping it sets
   `meta.cloud_save` and persists (see persistence). Hidden for the synthetic
   cloud-only entries (they're already stashed; gate on `created_at.is_empty()`
   which only the synthetic literal leaves blank).

3. **Pause menu** (`draw_pause_menu`): a small status line next to "Save and
   Quit" — "📦 Stash: On" / "Stash: Off" so the player knows, at save time,
   whether this session will be stashed. (Status only in Layer 1; flipping it
   here is optional polish, deferred.)

## Persistence

- **Native:** `save_world_meta` writes `world_meta.json` durably — the lobby
  toggle calls it directly (mirrors the Edit dialog at `menu.rs`).
- **WASM:** `save_world_meta` only updates an in-memory cache, and
  `world_store.js list()` projects a subset of meta. Two JS additions:
  - `list()` includes `cloud_save: !!(r.meta && r.meta.cloud_save)`.
  - new `axenstax_set_cloud_save(pubkey, name, bool)`: read-modify-write the
    record's `meta.cloud_save` in IndexedDB (the full meta object is already
    stored by `save()`), exposed via a `wasm_save.rs` extern. The lobby toggle
    calls it; menu list-mapping reads `e.cloud_save`.

## Testing

- **Unit (pure):** `should_stash` truth table; `WorldMeta` serde round-trip with
  a legacy blob missing `cloud_save` ⇒ `false`; new-world default ⇒ `false`.
- **`check.sh`:** clippy + build + `cargo test` + trunk build + bundle gate.
- **Playtest boundary (browser, owner-run):** create world with Stash OFF → save
  → no toast, not in Stash on a second device; flip ON in lobby → save → toast +
  appears on second device; flip OFF → stays on second device (no delete),
  marked out of sync.

## Out of scope / follow-on

- **Layer 2:** explicit "Remove from Stash", out-of-sync indicator + last-write
  conflict UX, optional "don't keep a local copy on shared devices" mode.
- **Layer 3:** Nostr provenance chain, BTC timestamp, sharing/forking,
  collaborative servers.
