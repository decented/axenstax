# World Integrity & Game Modes — Spec

## Goal

Add a World Integrity Ledger, game mode selection at creation, basic creative mode toggle, difficulty setting, and fork integrity inheritance. All metadata and UI — no new gameplay systems beyond what exists (flying, invulnerability, instant-break).

## What's In Scope

### 1. World Integrity Ledger

New fields on `WorldMeta` (persisted in `world_meta.json`):

| Field | Type | Default | Behaviour |
|-------|------|---------|-----------|
| `pure_survival` | `bool` | `true` | One-way door → `false` if creative or cheats ever used |
| `ever_creative` | `bool` | `false` | One-way door → `true` if creative mode ever activated |
| `cheats_used` | `bool` | `false` | One-way door → `true` if any command ever used |
| `difficulty` | `String` | `"normal"` | Current difficulty: `peaceful`, `easy`, `normal`, `hard` |
| `difficulty_history` | `Vec<DifficultyChange>` | `[]` | Append-only log of difficulty changes |
| `forked_from` | `Option<String>` | `None` | Parent world folder name (set on fork) |

`DifficultyChange`: `{ level: String, timestamp: String }`

One-way doors: once `pure_survival` is `false`, it never returns to `true`. Same for `ever_creative` and `cheats_used`. These are facts about the world's history.

Old worlds without these fields get defaults (pure_survival=true, ever_creative=false, etc.) via serde `#[serde(default)]`.

### 2. Game Mode at World Creation

The Create World dialog gains a mode selector: **Survival** (default) or **Creative**.

- Survival: standard gameplay (current default)
- Creative: sets `game_mode: "creative"`, `ever_creative: true`, `pure_survival: false`

Hardcore and Adventure are Phase 3 features — not included here.

### 3. Basic Creative Mode (In-Game)

When `game_mode == "creative"`:
- Player takes no damage (invulnerable)
- Flight always available (no double-tap needed — flying starts on)
- Block breaking is instant (no tool required, no durability cost)
- Blocks broken don't go to inventory (already have unlimited)
- All blocks available in hotbar (pre-filled with all block types)

This uses EXISTING systems: `combat.health` check, `player.flying` flag, the block-break code path. No new UI (unlimited inventory browser is Phase 3).

### 4. Creative Mode Toggle (Pause Menu)

In survival worlds, the pause menu gains a "Switch to Creative" button. Clicking it:
1. Shows confirmation: "This world will be permanently marked as creative-touched."
2. On confirm: sets `game_mode = "creative"`, `ever_creative = true`, `pure_survival = false` in WorldMeta
3. Saves the updated metadata immediately
4. Applies creative mode effects

There is NO toggle back to survival in this phase. That's a Phase 3 feature when full mode-switching exists.

### 5. Difficulty Setting (Pause Menu)

The pause menu gains a difficulty selector: Peaceful / Easy / Normal / Hard.

Current phase behaviour:
- **Peaceful**: mobs don't attack the player
- **Easy/Normal/Hard**: cosmetic distinction only (mob damage scaling is Phase 3)

Every difficulty change is logged to `difficulty_history` in WorldMeta. The setting persists across sessions.

### 6. Fork Integrity Inheritance

When forking a world:
- Copy ALL integrity fields from parent to fork
- Set `forked_from` to the parent's folder name
- The fork's `display_name` and `created_at` are new, but integrity history carries over

### 7. Menu Badge

World cards in the menu show a badge: "SURVIVAL" (green) or "CREATIVE" (blue). Already exists — just needs to read from actual game_mode.

## What's NOT In Scope

- Hardcore mode (needs permadeath system)
- Spectator mode (needs no-clip camera separate from player)
- Adventure mode (needs block permission tags)
- Difficulty affecting mob damage/hunger (Phase 3)
- Unlimited inventory browser UI (Phase 3)
- Mode switching back from creative to survival (Phase 3)
- Bitcoin reward rules tied to integrity (Phase 5)
- Server policy configuration (Phase 2)
