# World Management Menu — Design Spec

**Date:** 2026-03-31
**Status:** Approved
**Scope:** Main menu world list, new world creation, world actions (rename, edit, fork, delete), pause menu simplification

---

## Problem

The current main menu is a flat list of folder names. To delete a world you must enter it first and use the pause menu. There's no way to rename worlds, see metadata, or manage worlds without loading them. The pause menu has a "Delete World" button with no confirmation — one misclick destroys everything.

## Solution

Replace the flat world list with rich world cards showing metadata. Add world management actions (rename, edit description, fork, delete) directly from the main menu. Add a type-to-confirm deletion flow. Simplify the pause menu by removing delete.

---

## 1. World Metadata

### 1.1 Storage

New file `world_meta.json` alongside existing `world.dat` in each world directory:

```
worlds/<folder>/
  world.dat          — existing: bincode save data (seed, player, inventory)
  world_meta.json    — NEW: display name, description, game mode, dates
  chunks/            — existing: chunk files
```

### 1.2 Schema

```json
{
  "display_name": "My Castle World",
  "description": "A massive castle project with underground mines",
  "game_mode": "survival",
  "created_at": "2026-03-28T14:30:00Z",
  "icon": null
}
```

Fields:
- `display_name` — what the player sees. Independent of the folder name on disk.
- `description` — optional short text. Shown on the world card.
- `game_mode` — `"survival"` or `"creative"`. Displayed as a badge. Default: `"survival"`.
- `created_at` — ISO 8601 timestamp. Set once at creation.
- `icon` — reserved for future use (custom world icon/thumbnail). Always `null` for now.

### 1.3 Backward Compatibility

Existing worlds without `world_meta.json` get auto-generated defaults on first scan:
- `display_name` = folder name (e.g., `"default"`, `"world_3"`)
- `description` = `""` (shown as italic "No description" in UI)
- `game_mode` = `"survival"`
- `created_at` = filesystem creation date of `world.dat` (or current time if unavailable)

The auto-generated metadata is **not** written to disk until the player edits something. This avoids polluting existing world directories until the player takes action.

### 1.4 Derived Data (Not Stored)

Read from the filesystem at menu load time, not stored in metadata:
- **Last played** — `world.dat` last-modified timestamp
- **World size** — sum of all files in the world directory (displayed as "27 MB" etc.)

---

## 2. Main Menu — World List

### 2.1 Layout

Each world is a card showing:
- Display name (large, white text)
- Game mode badge (green "SURVIVAL" or blue "CREATIVE", pill-shaped)
- Description (small grey text, or italic "No description")
- Last played (relative: "2 hours ago", "Yesterday", "3 days ago")
- World size (e.g., "27 MB")

Cards are sorted by last played (most recent first).

### 2.2 Selection & Actions

- **Click** a card to select it. The selected card highlights (blue border) and reveals an action bar at the bottom of the card.
- **Double-click** a card to play immediately (shortcut).
- Action bar buttons: **Play** (green, prominent), **Edit** (opens rename + description dialog), **Fork**, **Delete** (red, right-aligned).

### 2.3 "Create New World" Button

Below the world list. Dashed border, "+ Create New World" text. Opens the creation dialog.

### 2.4 Empty State

If no worlds exist, show a centred message: "No worlds yet" with a prominent "Create Your First World" button.

---

## 3. Create New World Dialog

Modal overlay with two fields:

- **World Name** — text input, required. Placeholder: "My New World". This becomes both the `display_name` in metadata and the sanitised folder name on disk.
- **Seed** — text input, optional. Placeholder: "Leave blank for random". If provided, hashed to u32 for world generation.

Folder name derived from display name: lowercased, spaces to underscores, non-alphanumeric stripped, truncated to 32 chars. If collision, append `_2`, `_3`, etc.

Buttons: **Create & Play** (green, primary), **Cancel**.

On create: write `world_meta.json`, then enter the world normally.

---

## 4. World Actions

All actions available from the main menu without entering the world.

### 4.1 Play

Load and enter the world. Same as current behaviour.

### 4.2 Edit

Single modal dialog for editing world metadata:
- **Name** — text input, pre-filled with current display name.
- **Description** — text area, pre-filled with current description (or empty).

Edit changes `display_name` and `description` in `world_meta.json` only. The folder name on disk does not change. This avoids path reference issues and keeps save compatibility.

Buttons: **Save** (blue, primary), **Cancel**.

### 4.3 Fork

Duplicates the entire world directory to a new folder. The new world gets:
- `display_name` = `"<original name> (fork)"`
- `description` = `"Forked from <original name>"`
- `created_at` = now
- All chunks and world.dat copied as-is

The new world appears in the list immediately. The original is untouched.

Fork is a potentially slow operation (copying 20-30 MB of chunk files). Show a brief "Forking world..." indicator. No modal — just a status message on the card.

### 4.4 Delete

**Requires type-to-confirm.** Modal dialog showing:

1. Warning icon and "Delete World" title in red
2. Message: "This will permanently delete **"My Castle World"** and all its data. This cannot be undone."
3. Text input: "Type **My Castle World** to confirm"
4. **Delete Forever** button — **disabled** until the typed text matches the display name exactly (case-sensitive)
5. **Cancel** button

This pattern prevents accidental deletion. It's intentionally harder than a yes/no dialog — especially important given Axolittle's age. Muscle memory can't delete a world.

On confirm: `fs::remove_dir_all` the world directory. Return to world list (which re-scans).

---

## 5. Pause Menu — Simplified

Remove "Delete World" from the pause menu entirely. The revised pause menu has three buttons:

1. **Resume** (green)
2. **Save** (blue)
3. **Save and Quit** (orange) — saves and returns to main menu

Delete belongs in the main menu where you can see all your worlds and make a deliberate choice, not in the heat of gameplay.

---

## 6. Menu State Machine

The `GameMode::Menu` variant needs to track UI state for dialogs:

```
Menu (world list)
  ├── CreateDialog { name: String, seed: String }
  ├── EditDialog { world_idx: usize, name: String, description: String }
  ├── DeleteDialog { world_idx: usize, confirm_text: String }
  └── Forking { world_idx: usize }  (brief, auto-dismisses)
```

Only one dialog open at a time. Esc closes the active dialog and returns to the world list. Esc from the world list quits the game.

---

## 7. Data Model (Rust)

```rust
/// Metadata for a saved world, stored in world_meta.json.
#[derive(Serialize, Deserialize, Clone)]
pub struct WorldMeta {
    pub display_name: String,
    pub description: String,
    pub game_mode: String,       // "survival" or "creative"
    pub created_at: String,      // ISO 8601
    pub icon: Option<String>,    // Reserved for future
}

/// A world entry as displayed in the menu (metadata + derived data).
pub struct WorldEntry {
    pub folder_name: String,     // Directory name on disk
    pub meta: WorldMeta,         // From world_meta.json (or auto-generated)
    pub last_played: SystemTime, // world.dat mtime
    pub size_bytes: u64,         // Total directory size
}
```

---

## 8. Files Affected

| File | Change |
|------|--------|
| `save.rs` | Add `WorldMeta` struct, `load_world_meta()`, `save_world_meta()`, `list_world_entries()`, `fork_world()`, `sanitize_folder_name()` |
| `menu.rs` | Rewrite `draw_main_menu()` with cards, selection, action bar. Add dialog draw functions. Remove old `hit_test` functions (already gone). |
| `main.rs` | Update `GameMode::Menu` to hold `Vec<WorldEntry>` and dialog state. Update event handling for new menu interactions. |
| `game_loop.rs` | Update menu rendering to pass `WorldEntry` data. Handle new `MenuAction` variants. |
| Pause menu in `menu.rs` | Remove delete button. 3 buttons only. |

---

## 9. Out of Scope

- Custom world icons/thumbnails (reserved in schema, not implemented)
- World export/import
- World settings (difficulty, game rules)
- Search/filter for many worlds
- Drag-to-reorder

These can be added later without schema changes.
