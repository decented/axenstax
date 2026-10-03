# WorldEdit-style region editing — the creative-power backbone

**Status**: ✅ BUILT 2026-06-16 (worktree `worktree-community-features`, goal `2026-06-16-community-features-solo-buildout`). Phase 1 feature **#7** (replaces WorldEdit / Axiom sculpting basics). The creative backbone + a concrete "bring your building workflow here" hook for the builder/modder outreach audience.
**Date**: 2026-06-16
**Backlog**: `docs/research/2026-06-15-native-bake-in-feature-backlog.md` §7.

---

## TL;DR

Region selection + mass-edit commands on the existing command system: pick two corners, then `set` / `replace` / `walls` / `copy` / `paste` / `stack` over the whole cuboid in one command. Pure region ops over the `World`; a per-player selection + clipboard session on `PlayerSlot`. Operates fully in single-player / creative.

## Surface (the `/we` command group)

The command parser strips a single `/`, so WorldEdit's `//set` double-slash syntax isn't usable — it's one `/we` group instead (`OpLevel::Op`, cheat-flagged):

- `/we pos1` · `/we pos2` — set a selection corner at your feet.
- `/we set <block>` — fill the selection.
- `/we replace <from> <to>` — swap one block for another inside the selection.
- `/we walls <block>` — the four vertical side faces of the selection.
- `/we copy` · `/we paste` — clipboard the selection / stamp it at your feet (min corner).
- `/we stack <n> [x|y|z]` — repeat the selection `n` times along an axis (default x).
- `/we size` · `/we clear` — report block volume / clear the selection.

## Design (concrete, not cards)

- **`worldedit.rs`** — pure mutation ops over `&mut World` (`region_set`/`region_replace`/`region_walls`/`region_copy`/`clipboard_paste`/`region_stack`) + helpers (`bounds`/`volume`/`affected_chunks`). All headless-unit-tested (8 tests). `Clipboard { dims, blocks }` stores ids relative to the min corner (x-fastest).
- **`WorldEditSession`** (`{ pos1, pos2, clipboard }`) on `PlayerSlot` — transient, not persisted.
- **Safety**: `MAX_REGION_VOLUME` (2,000,000) caps every op (and `stack`'s projected result) so a runaway `/we set` can't hang/OOM.
- **Re-mesh**: edits return `CommandResult::RebuildRegion { min, max, changed }`; the game loop marks every chunk in `worldedit::affected_chunks(min, max)` dirty → the existing batched chunk re-mesh updates the world visually.
- **Gameplay-grade cell writes (audit fix 2026-09-28).** Every op writes through `worldedit::write_cell`, not the raw `World::set_block`: the old block's meta and block-entity go with it (a `/we set air` over a battery used to leave its `PowerDevice` as a ghost source; a chest's contents stayed orphaned), a power block gets its `PowerDevice` registered (a pasted lamp-and-lever build used to be inert), and power cells wake their network (`mark_dirty` + `notify_neighbours`). Only power cells notify — six neighbour enqueues per cell of a 2M-cell region would flood the scheduler. Fluid bookkeeping is still not done (a `/we set air` over water leaves its source registered).
- **Multiplayer (audit fix 2026-09-28).** Ops report every changed cell in `RebuildRegion.changed`; when hosting, the game loop applies them to the hosted server's world (`apply_remote_block_change`) and queues the cells for broadcast, drained at most 1024 per StateUpdate in order (one oversized StateUpdate was dropped whole by joiners), so joiners and the server sim see the edit. On a joined client `/we` is refused (joiners dispatch at `OpLevel::None`).

## Solo boundary → playtest gate

Solo: every region op + the command routing + the re-mesh trigger are headless-testable (12 tests). **Playtest**: ergonomics + the deferred items below.

## Deferred (named — not placeholders)

- **In-world selection box rendering** (a translucent cuboid outline showing the current selection) — needs the world-space overlay buffer (shared with #8). Today you confirm the selection via `/we size`.
- **A selection wand** (left/right-click a tool to set corners) — needs item/click wiring; commands cover it for now.
- **Brushes** (`//brush sphere`, etc.), `//rotate` / `//flip` of the clipboard, `//sphere` / `//cyl` shapes, `//count` / `//distr`, undo/redo history. Each builds on the same pure-op + RebuildRegion seam.

## Spec maintenance

Spec 05 (Gameplay) creative-tools note + the engine-commands foundation reference the `/we` group.
