# Minecraft schematic import — bring your MC builds across

**Status**: ✅ BUILT 2026-06-16 (worktree `worktree-community-features`, goal `2026-06-16-community-features-solo-buildout`). Phase 1 feature **#10** — a direct acquisition lever for the MC builder/modder outreach audience: own the *import* so they can bring their builds in.
**Date**: 2026-06-16
**Backlog**: `docs/research/2026-06-15-native-bake-in-feature-backlog.md` §10.

---

## TL;DR

`/import <path-to.schem>` reads a Minecraft **Sponge `.schem`** (gzipped NBT) and converts it into an AxeNStax `PlanData` blueprint, added to the plan registry and immediately usable with **`/buildguide <name>`** (the #9 build-along). The block palette is mapped best-effort: known MC blocks → their AxeNStax equivalent, air skipped, anything unmapped → `STONE` so the **shape** survives.

## Design (concrete, not cards)

- **`nbt.rs`** — a hand-rolled minimal big-endian **NBT reader** (no new crate; `flate2`/gzip is already a dependency). Parses the subset `.schem` needs (Compound/List/String/Short/Int/ByteArray/IntArray/Long…). 4 unit tests over hand-built byte blobs (incl. truncation = `Eof`, bad-tag).
- **`schematic.rs`** — `parse_schem(gzipped, name)`: gunzip → NBT-parse → `schem_to_plan`. Handles Sponge **v2** (`Palette` + `BlockData` at the schematic level) and **v3** (`Blocks { Palette, Data }`, nested under `Schematic`). `decode_varints` (LEB128 palette indices), `map_mc_block` (MC block-state → `BlockId`), YZX index→coords. 4 unit tests (varints, mapping, end-to-end over a hand-built NBT tree, oversize reject). Dimensions are `u8` (`PlanData` limit) — oversize (>255/axis) is rejected.
- **`PlanData::from_imported`** — builds a finished, Master, Developed, Building-kind plan with a fresh content-hash derivation link; `AllRightsReserved` (imported builds carry no original-author grant — a tag, not a gate).
- **`PlanRegistry::add`** — runtime add (replaces a same-name plan, so re-import updates in place).
- **`/import`** (`OpLevel::None`, not a cheat) — native file read → `parse_schem` → registry add. WASM returns "native-only" (the in-page `<input type=file>` picker is a follow-up).

## Solo boundary → playtest gate

Solo: NBT parse, varint decode, block mapping, schem→plan, registry add — all headless-tested (8 tests). **Playtest** (owner): import a real `.schem` from a MC build, check the shape + the block-mapping quality, then `/buildguide` it.

## Deferred (named)

- **`.litematic`** (Litematica's packed-long-array format — different + more complex). `.schem` is the common interchange format; `.litematic` is its own parser.
- **WASM in-page import** (`<input type=file>` → bytes → `parse_schem`).
- **A native file-picker** trigger (vs the `/import <path>` command) + a richer **block-mapping table** (more 1:1 mappings; stairs/slabs once #30 block-shapes land; wool/concrete colours).
- **Round-trip export** (AxeNStax → `.schem`).

## Spec maintenance

Spec 05 (Gameplay) creative-tools note references `/import`; the format + mapping limits are captured here.
