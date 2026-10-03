# Engine commands — chat overlay + parser + dispatcher (plugin-shaped)

**Status**: Phases 1–4 DELIVERED 2026-05-08 on main. Commits `0f7a269` (chat overlay + parser + dispatcher + 7 built-ins, plugin-shaped) → `b479099` (self-review pass) → `d864fe4` (chat input gate fix) → `909a0a9` (clippy clean), with `bfd8cd5` threading the feature through reference surfaces. Folds the world-time 4× BRIDGE into `/time speed`. Phase 5 (multiplayer dispatch via `hosted_server.rs`) deferred behind Specs 1+2 — must NOT parallelise on `hosted_server.rs`. Phase 6 (Axolittle UX playtest) blocked on his time. Test count: 65 new tests, 236 total. `check.sh` ALL GREEN.
**2026-05-30 fix (day↔night swap)**: `/time set day` was turning it dark and `/time set night` bright. Root cause: the keyword→tick table in `commands/builtins/time.rs` was copied from Minecraft's clock (`day=1000, night=13000`), but this engine's clock (`camera.rs::compute_sun`) is offset +6000 — **`0 = midnight, 6000 = sunrise, 12000 = noon, 18000 = sunset`** (sun above horizon for ticks `6000..=18000`). Corrected the table to Minecraft semantics + 6000 (mod 24000): `day=7000, noon=12000, night=19000, midnight=0`. Extracted a pure `time_for_keyword()` so the mapping is unit-tested against the clock convention (behavioural test, not a literal constant, so it can't silently re-encode a swap). **The clock convention above is the source of truth — keep `time.rs` and `camera.rs` in agreement.**
**Date**: 2026-05-07 (drafted) → 2026-05-08 (Phases 1–4 landed)
**Branch**: `feat/engine-commands` off `main`, merged.
**Session**: Fresh — implementer should treat this doc as the only brief.

---

## TL;DR

The engine has zero command infrastructure today (`grep -rn "command|chat_input|process_command" game/engine/src/` returns only wgpu's render command-encoder). This spec adds:

1. A **chat overlay** (egui) that opens on `T`, captures keystrokes from gameplay while open, and renders an output log that fades after a few seconds.
2. A **tokeniser + parser** that turns `/time set 6000` into a structured `ParsedCommand { name: "time", args: ["set", "6000"] }`.
3. A **trait-based registry** — each command implements `Command`. Built-in registrations: `/help`, `/time`, `/gamemode`, `/tp`, `/seed`, `/clear`, `/give`. Game-specific games (other games on the same primitives) register their own.
4. A **single-player dispatcher** that runs commands against the local `GameState`. Multiplayer dispatch is Phase 5, gated behind the Spec 1 + Spec 2 `hosted_server.rs` queue (CLAUDE.md: don't parallelise on that file).
5. Hooks into the **World Integrity Ledger** — commands that mutate world state (`/gamemode`, `/give`, `/time set`) set `cheats_used`. Read-only commands (`/help`, `/seed`, `/time get`) don't.

Designed plugin-shaped: registry + parser + UI are **game-agnostic**. Built-in commands are voxel/Minecraft-style but can be replaced wholesale by another game keeping the same registry. When the engine grows `src/lib.rs` (likely for the PWA/native split or for a `genesis_server` crate), the `commands::` module lifts cleanly to its own crate (`axenstax-commands` or, if generalised, `voxel-commands`) with no API breakage.

Total scope: ~1100 lines (spec → tests → UI → built-ins). 4 autonomous phases (spec + parser/registry + UI + built-ins). Phases 5–6 are blocked on hosted_server.rs queue + Axolittle playtest respectively.

---

## Why this lives here

- Per shared infra strategy (memory): every game in the portfolio benefits from a chat + command surface. Designed for cross-game lift.
- Per qr signin regression runbook (memory) precedent: instrumentation is force-multiplier. A `/debug` command will save evening-long investigations next time something breaks in-engine.
- Per the user's day-length question (decision 5 trigger, 2026-05-07): `/time speed <n>` is the right tool for "I want the day longer/shorter without rebuilding". Replaces both the 4× BRIDGE *and* the temptation to thread a `WorldMeta.time_speed` field that'd need its own UI.

---

## Context pointers

- Engine entry: `game/engine/src/main.rs` — `GameMode::Playing` is the active state during sim.
- Input handling: `game/engine/src/input.rs:51` — `PlayerIntent` + `hotbar_select`. Chat will need to **gate** these while open.
- HUD overlay: `game/engine/src/hud_ui.rs:35` — `draw_debug_overlay` pattern. Chat overlay sits in the same `egui::Context` slot.
- World time: `game/engine/src/main.rs:187` — `world_time: u32` on `GameState`. Currently advances at `+ 4 % 24000` per tick (BRIDGE in `game_loop.rs:60` + `server.rs:283`). `/time speed <n>` overrides; `/time set <day|night|N>` jumps.
- World Integrity Ledger: `game/engine/src/save.rs` — `WorldMeta.cheats_used`, `ever_creative`, `pure_survival`. One-way flags. `/gamemode` and `/give` set them.
- Game mode flip: `game/engine/src/main.rs` — `GameMode::Paused { confirm_creative: bool }` already exists for the Esc-menu path. `/gamemode` reuses that flow's mutation logic, skips the confirm dialog (the command IS the confirmation).
- Save format: `WorldMeta` is `bincode`-serialized; new fields require a versioned migration path. **No `WorldMeta` schema changes in this spec** — all command state is per-session except `cheats_used` which already exists.

---

## Scope

| # | Phase | Files | Est. lines | Autonomous? |
|---|-------|-------|:---:|:---:|
| 1 | This spec doc | `docs/foundations/2026-05-07-engine-commands.md` | 600 | ✓ |
| 2 | Parser + registry + dispatcher (no UI) | `src/commands/{mod,parser,registry,dispatch}.rs` + tests | 350 | ✓ |
| 3 | Chat overlay UI + input gating | `src/chat_ui.rs`, `src/main.rs`, `src/input.rs`, `src/game_loop.rs` | 250 | ✓ (single-player only) |
| 4 | Built-in commands (`/help`, `/time`, `/gamemode`, `/tp`, `/seed`, `/clear`, `/give`) | `src/commands/builtins/{help,time,gamemode,tp,seed,clear,give}.rs` + tests | 400 | ✓ |
| 5 | Multiplayer command propagation | `hosted_server.rs`, `protocol.rs`, `remote_client.rs` | 150 | **Blocked** by Spec 1 Phase 4 + Spec 2 Phase 0b sharing `hosted_server.rs` |
| 6 | Axolittle playtest — UX + command surface call | n/a | 0 | **Blocked** by Axolittle availability |

**Total**: ~1150 lines. Phases 1–4 fully autonomous and ship as `feat/engine-commands` without touching the `hosted_server.rs` queue. Phase 5 sits behind the Spec 1+2 work in the queue. Phase 6 is the playtest gate.

---

## Phase 2 — Parser + registry + dispatcher

### File: `game/engine/src/commands/mod.rs` (new)

```rust
//! Engine command system — chat-driven slash commands.
//!
//! Plugin-shaped: registry + parser + dispatcher are game-agnostic. Built-in
//! commands (in `builtins::`) are voxel/Minecraft-style; another game can
//! register its own without touching this module.

pub mod parser;
pub mod registry;
pub mod dispatch;
pub mod builtins;

pub use parser::{ParsedCommand, ParseError};
pub use registry::{Command, CommandRegistry, OpLevel};
pub use dispatch::{CommandContext, CommandResult, dispatch};
```

### File: `game/engine/src/commands/parser.rs` (new)

Tokeniser. Strips leading `/`, splits on whitespace, supports `"quoted args with spaces"` and `\\` escape. Returns `Err(ParseError::Empty)` for `/` alone, `Err(ParseError::UnclosedQuote)` for stray `"`.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedCommand {
    pub name: String,         // "time" (always lower-case, no leading /)
    pub args: Vec<String>,    // ["set", "6000"]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    NotACommand,        // input doesn't start with `/`
    UnclosedQuote,
    OnlySlash,
}

pub fn parse(input: &str) -> Result<ParsedCommand, ParseError> { … }
```

### File: `game/engine/src/commands/registry.rs` (new)

Trait `Command` + `CommandRegistry`. Registry stores `Box<dyn Command>` keyed by primary name; aliases also resolve to the same command. Iteration order = registration order (for `/help` listing).

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OpLevel { None = 0, Op = 1 }

pub trait Command: Send + Sync {
    fn name(&self) -> &'static str;
    fn aliases(&self) -> &'static [&'static str] { &[] }
    fn help(&self) -> &'static str;
    fn usage(&self) -> &'static str;
    fn min_op_level(&self) -> OpLevel { OpLevel::Op }
    /// True if executing this command should set WorldMeta.cheats_used.
    fn is_cheat(&self) -> bool { true }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult;
}

pub struct CommandRegistry { … }
impl CommandRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, cmd: Box<dyn Command>);
    pub fn lookup(&self, name: &str) -> Option<&dyn Command>;
    pub fn iter(&self) -> impl Iterator<Item = &dyn Command>;
}
```

### File: `game/engine/src/commands/dispatch.rs` (new)

`CommandContext` is the **only** API surface commands see. Bounded to keep the trait reusable across games:

```rust
pub struct CommandContext<'a> {
    pub world: &'a mut crate::chunk::World,        // for /tp, /setblock
    pub state: &'a mut crate::main::GameState,     // for /time, /gamemode (world_time, mode flip)
    pub player_idx: usize,                          // who ran the command
    pub registry: &'a CommandRegistry,             // for /help to enumerate
    pub op_level: OpLevel,                         // current player's op level
    pub log: &'a mut Vec<ChatLine>,                // append output here
}

#[derive(Clone, Debug)]
pub enum CommandResult { Success, Error(String), Silent }

#[derive(Clone, Debug)]
pub struct ChatLine {
    pub text: String,
    pub kind: ChatLineKind,
    pub at_tick: u32,
}

#[derive(Clone, Copy, Debug)]
pub enum ChatLineKind { Info, Echo, Success, Error, System }

pub fn dispatch(input: &str, ctx: &mut CommandContext, registry: &CommandRegistry) -> CommandResult {
    // 1. parse(input) → ParsedCommand or push parse error to log + return Error
    // 2. registry.lookup(name) → command or "unknown command" error
    // 3. permissions check: ctx.op_level >= cmd.min_op_level() else "you don't have permission"
    // 4. cmd.execute(ctx, &parsed.args)
    // 5. if cmd.is_cheat() and result is Success, set WorldMeta.cheats_used (via state.world_meta or whatever the path is)
    // 6. return result
}
```

### Phase 2 tests (`src/commands/tests.rs` — `#[cfg(test)] mod tests`)

Parser:
- `parse("/time set 6000")` → `ParsedCommand { name: "time", args: ["set", "6000"] }`
- `parse("/say \"hello world\"")` → `ParsedCommand { name: "say", args: ["hello world"] }`
- `parse("/say \"unclosed")` → `Err(UnclosedQuote)`
- `parse("/")` → `Err(OnlySlash)`
- `parse("hello")` → `Err(NotACommand)`
- `parse("")` → `Err(Empty)`
- `parse("/TIME GET")` → name lowercased to `"time"`, args preserved as-is `["GET"]` (commands lower-case the name; arg case is command's choice).

Registry:
- Register two commands with overlapping aliases → second registration wins for the alias.
- `lookup("missing")` → None.
- `iter()` returns commands in registration order.

Dispatch:
- Unknown command → `CommandResult::Error("unknown command: foo")`, log gets one Error line.
- Permission denied (player op_level < cmd.min_op_level) → Error, log gets one Error line.
- Successful cheat command bumps `cheats_used`; non-cheat command doesn't (verify via mock `WorldMeta`).

### Acceptance — Phase 2

- `cargo test --bin axenstax-engine commands::` green; minimum 12 new tests across parser/registry/dispatch.
- `cargo clippy -- -D warnings` clean for `commands::*`.
- No use of `unwrap()` / `panic!()` on user input — all paths return `ParseError` or `CommandResult::Error`.

---

## Phase 3 — Chat overlay UI + input gating

### File: `game/engine/src/chat_ui.rs` (new)

egui overlay, anchored bottom-left. Two regions:

1. **Output log** — last 8 lines, semi-transparent background, fades each line after 8s of in-game time.
2. **Input field** — only visible when `chat_open == true`. Prepended `>` prompt. Auto-focus on open.

Public API:
```rust
pub struct ChatState {
    pub open: bool,
    pub input: String,
    pub log: Vec<ChatLine>,
    pub history: Vec<String>,        // submitted commands, for ↑/↓ navigation
    pub history_cursor: Option<usize>,
}

impl ChatState {
    pub fn open(&mut self);
    pub fn close(&mut self);
    pub fn submit(&mut self) -> Option<String>; // returns input + clears + closes
}

pub fn draw_chat(ctx: &egui::Context, state: &mut ChatState, current_tick: u32) -> ChatAction;

pub enum ChatAction { None, Submit(String), Cancel }
```

### File: `game/engine/src/input.rs` (existing, modify)

Add `chat_open` gate: if `chat_open`, **all** key events go to the chat field via egui's text-input handling. Movement intents (`forward`, `back`, `left`, `right`, `jump`, `crouch`), hotbar select, and break/place buttons are forced to `false` / `None`.

```rust
pub fn build_intent(/* …existing args… */, chat_open: bool) -> PlayerIntent {
    if chat_open { return PlayerIntent::idle(); }
    /* …existing logic… */
}
```

`T` opens chat (matches Minecraft). `/` opens chat AND pre-fills the input with `/` (also Minecraft convention). `Esc` while chat is open closes without submitting. `Enter` submits + closes. `↑` / `↓` cycles `history`.

Mouse stays free — no cursor capture changes while chat open.

### File: `game/engine/src/main.rs` (existing, modify)

`GameState` gains:
```rust
pub(crate) chat: chat_ui::ChatState,
pub(crate) cmd_registry: commands::CommandRegistry,
```

Initialised at `GameState::default()` with `commands::builtins::register_all(&mut registry)`.

Game-loop tick (`game_loop.rs:60` area): if a `ChatAction::Submit(line)` came back from `draw_chat`, build a `CommandContext`, call `commands::dispatch(&line, &mut ctx, &state.cmd_registry)`. Append `ctx.log` results to `state.chat.log`.

### Acceptance — Phase 3

- WASM build green (`cd game/engine && trunk build --release`).
- `T` opens chat in single-player. Player stops moving while open. `Esc` closes. `Enter` submits.
- `↑` / `↓` walks history.
- Output log fades after 8s.
- No clippy warnings introduced.
- Manual smoke (Claude can do this in a headless playwright pass against the lobby): paste `T` keypress event, paste `/help`, press Enter, assert log contains "Available commands".

---

## Phase 4 — Built-in commands

Each command is one file in `src/commands/builtins/`. All implement `Command`. Registration order = listing order in `register_all`.

### `help.rs`
- Aliases: `?`
- `min_op_level: None` (anyone can read help)
- `is_cheat: false`
- Args:
  - `/help` → list all commands the player has permission to run, with usage line each
  - `/help <command>` → show that command's help text + usage
- Implementation reads `ctx.registry.iter()`.

### `time.rs`
- `min_op_level: Op`
- `is_cheat: true` for set/speed; `false` for get
- Subcommands:
  - `/time set <day|night|noon|midnight|N>` — sets `state.world_time` (modulo 24000)
    - `day` = 1000, `noon` = 6000, `night` = 13000, `midnight` = 18000
  - `/time speed <N>` — sets `state.world_time_step` (new field on GameState, default 4 to match alpha; later default 1 = standard 20 min)
  - `/time get` — prints "Time: HH:MM (tick N), speed=Nx"

**Side effect**: this lets the user replace the World time 4× speed BRIDGE without a rebuild. Set the default to 4 today; flipping the default to 1 is a one-line change later. AS-007 is folded into this command.

### `gamemode.rs`
- Aliases: `gm`
- `min_op_level: Op`
- `is_cheat: true`
- Subcommands:
  - `/gamemode survival` (`gm s`) → flip to survival, reset `state.is_creative = false`, set `pure_survival = false` if was creative
  - `/gamemode creative` (`gm c`) → flip to creative, set `is_creative = true`, set `ever_creative = true`, `pure_survival = false`
- Reuses the same flip path as the Esc-menu's "Switch to creative" confirmation — skips the dialog (the command IS the confirmation).

### `tp.rs`
- `min_op_level: Op`
- `is_cheat: true`
- `/tp <x> <y> <z>` → teleport self
- `/tp <player>` (deferred to multiplayer; Phase 5 wire-up)
- Validates coords are finite; clamps to world bounds.

### `seed.rs`
- `min_op_level: None`
- `is_cheat: false`
- `/seed` → prints the world seed (read from `state.biome_gen.seed` or wherever it lives).

### `clear.rs`
- `min_op_level: Op`
- `is_cheat: true`
- `/clear` → empties player inventory
- `/clear <item>` (deferred — needs item-name lookup table; Phase 4.5)

### `give.rs`
- `min_op_level: Op`
- `is_cheat: true`
- `/give <item> [count]` — adds to player inventory
- For v1: hard-coded item-name map (`stone`, `dirt`, `oak_log`, `cobblestone`, `wooden_pickaxe`, `wooden_sword` …). Generated from `block::*` constants.
- Future: same lookup table also powers `/clear <item>` and a creative item picker (would obsolete the chunk_stream creative-blocks setup).

### Phase 4 tests

Each command gets a `#[cfg(test)] mod tests` block. Use a mock `CommandContext` constructed in the test (fields exposed pub(crate) for testing).

- `time::set` parses `day|night|noon|midnight` and integer
- `time::set` rejects out-of-range integers (`/time set 99999`) → Error
- `time::speed` accepts 1..=64, rejects 0 and >64
- `gamemode::survival` flips state, sets pure_survival false if was creative
- `gamemode::creative` flips state, sets ever_creative true
- `tp` accepts three floats, rejects NaN, clamps
- `give::stone 5` adds 5 stone, returns Success
- `give::stone 999` (over stack size) caps at 64 (one stack), returns Success with "added 64 (capped)"
- `clear` empties all 36 slots
- `help` (no args) lists registered commands
- `help time` shows time's help text

### Acceptance — Phase 4

- `cargo test --bin axenstax-engine commands::builtins::` green.
- `./check.sh` green.
- WASM build green.
- Manual playwright smoke (Claude): in single-player, type `/help` → see command list; `/time set night` → sun moves; `/gamemode creative` → flying enabled.

---

## Phase 5 — Multiplayer command propagation (deferred)

> **Client half, as built (audit fix 2026-09-28).** Until Phase 5 exists, a command typed on a JOINED client runs locally at `OpLevel::None` (`world_exit::local_command_op_level`) — `/help`, feedback and the other non-op commands work; cheat-tier commands and `/trial` (it teleports, places beacons or grants a kit) are refused. In the player's own world it stays `Op`, with the world's Commands setting deciding whether chat opens. The World Integrity Ledger flags and `game_mode` now persist on **both** targets (the web writes the `WASM_META_CACHE` that `save_world` packs into the blob; before, `/gm c` on the web taster reverted on reload), and never into a joined session's local meta. On a host, `/time` reaches the server because the host pushes its clock every tick, and `/we` / `/killall` are mirrored into the hosted server (see Spec 04 §9.5).

**Blocked**: do not start until Spec 1 Phase 4 (`USE_SIGNET_AUTH=true`) and Spec 2 Phase 0b (`ENABLE_SINGLEPLAYER_HOSTED_SERVER=true`) have either landed or been explicitly unblocked. Both touch `hosted_server.rs`. CLAUDE.md: "Don't parallelise Specs 1+2 in flight — both touch hosted_server.rs. Either interleave by phase or serialise."

Adds:
- `ClientPacket::ChatCommand { text: String }`
- `ServerPacket::ChatLine { text: String, kind: ChatLineKind }`
- Server-side dispatch: parses, looks up player's op level (from a new `WorldMeta.ops: Vec<Hex64>` list — first Signet-authed player auto-op'd; manual op via `/op <pubkey>` later).
- Echoed output goes to all players if it's a world-mutating command (`/time`, `/gamemode`); private if it's `/help`, `/tp` self.

Spec deferred to a follow-on doc `docs/foundations/2026-XX-XX-engine-commands-multiplayer.md` once Phases 1–4 are validated single-player.

---

## Phase 6 — Axolittle playtest (deferred)

**Blocked**: Axolittle availability.

Single test sheet at `docs/test-sheets/<date>-engine-commands.md`, drafted on the day. Validates:
- `T` opens chat without breaking gameplay
- `/help` lists commands clearly
- `/time set night` produces expected world-time change
- `/gamemode creative` flips correctly + flying works
- `/give stone 64` adds stone to hotbar
- Error path: `/time set banana` produces a sensible error
- "Does the chat get in the way?" UX call

---

## Cross-game lift design

The registry + parser + UI are **AxeNStax-agnostic**. The only AxeNStax-specific code is:
- The built-ins themselves (because they assume blocks, voxel coords, world-time)
- The `CommandContext` field types (`World`, `GameState`)

To lift to another game (other games on the same primitives):
1. Replace `CommandContext` types with that game's state types (or generalise via a trait `GameContext`).
2. Reimplement `builtins/` for that game's mechanics.
3. Keep `parser.rs`, `registry.rs`, `chat_ui.rs` verbatim.

When the engine grows `src/lib.rs` (likely for the PWA/native target split), `commands::` moves to its own crate (`axenstax-commands` or, if generalised in step 1, `voxel-commands`). The `CommandContext` becomes a generic over a `GameContext` trait. No API breakage for the AxeNStax bin — just a different `Cargo.toml` import path.

**Don't** prematurely extract to a crate now. Engine is bin-only per CLAUDE.md; converting to a workspace is its own refactor.

---

## Acceptance — global

- `./check.sh` green at the end of every phase.
- `./check.sh --smoke` green at end of Phase 3 (chat opens and accepts input under playwright).
- `cargo test` ≥ 25 new tests across parser/registry/dispatch/builtins.
- Spec 05 §4 (Inventory & Crafting) gets a new sub-section §4.5 (Commands) referencing this doc and the built-in surface.
- `docs/sentinel/state.yaml` AS-007 (World time 4× speed BRIDGE) folds into `/time speed` and is marked done — no separate revert needed.
- Docs site (`tools/sites/docs/`) gets a Commands page so the reader can review what was built without reading source.

---

## Non-goals

- **Permissions UI / op management screen** — `WorldMeta.ops` field comes in Phase 5; UI for managing ops is a future polish item.
- **Tab completion / autocomplete** — would be lovely; deferred to Phase 7 polish.
- **Touch-input chat** — the on-screen-keyboard story is a separate problem. Touch users don't get commands in v1; document as Phase 7.
- **Scripting language / `/execute` chains** — Minecraft has these; we're explicitly not chasing parity. Each command is one verb.
- **Logging commands to disk** — chat history is in-memory only. If a command sets `cheats_used`, that's the durable record.
- **Voice-driven commands** — voice-server can transcribe but YAGNI for v1.
- **Server-only commands** (`/save`, `/stop`) — single-player only in v1; deferred to Phase 5.
