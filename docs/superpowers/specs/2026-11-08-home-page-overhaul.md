# Home page overhaul (2026-11-08)

The lobby ("home page", the screen after launch) grew one button at a time: a toolbar of eight equal buttons, an unexplained Reset beside The Workshop, a global Spawn dropdown, nine buttons on a selected world card (Delete among them), and a side panel that mixes "browse an npub" with two official races. The Trials gallery set the bar the rest should meet. This spec replaces the lobby's layout and names; it changes no game rule, save format or wire format.

Inventory of every control before the change (labels, actions, platforms, file:line): kept with the work as the starting point; the controls are listed again below with where each one goes.

## 1. Principles

1. **One obvious next thing.** The biggest thing on screen is "carry on where you left off".
2. **Few big destinations, not many small buttons.** Five places, each a large tile. Everything else lives inside the place it belongs to.
3. **An action sits on the thing it acts on.** Hosting, exporting, copying and deleting a world are on that world; importing worlds is on the world list; clearing the Workshop is on the Workshop.
4. **Rare and destructive actions go behind "More".** Delete is never a first-row button.
5. **Name things for what they do, in plain words.** No feature names a player has to learn first.
6. **Don't show what doesn't work.** A control that does nothing on this platform is hidden, not shown greyed or explained by an error.
7. **Same look as the Trials gallery:** dark ground, rounded cards with a colour band, the brand gold (`brand::LANTERN`) focus ring, `‹ Back` top-left on every inner page.
8. **Works on a TV.** See §2.

## 2. TV-safe area, size and focus (all lobby pages AND the Trials pages)

- **Safe area:** every lobby page, the Trials gallery, a trial's page (hero view), the Album and the island's trial card draw inside a safe rect inset by `max(16 pt, 5% of the screen's width)` left and right and `max(16 pt, 5% of the height)` top and bottom. One shared helper (`ui_safe_area`), used everywhere, with unit tests. Backgrounds still fill the screen; only content is inset. Dialogs are centred inside the safe rect and never larger than it.
- **Size:** body text at least 15 pt, tile titles 22–28 pt, page titles 30–36 pt; hit targets at least 44 pt tall. Large screens (safe width ≥ 1500 pt, e.g. a 1920×1080 TV) scale lobby type and tiles up by about 1.25 so they read from a sofa.
- **Focus:** the keyboard reaches everything: arrow keys move between tiles and cards in reading order, Enter activates, Escape goes back one page (never quits from an inner page). The focused item wears the gold ring.

## 3. Home

```
┌ safe area ─────────────────────────────────────────────────────────┐
│ AXE'N'STAX                              [● Guest ▾] [⚙ Settings] [Quit] │
│                                                                     │
│ ┌ Continue ───────────────────────────┐ ┌ Trials ─────────────────┐ │
│ │  [ world picture, 16:9 ]            │ │ [ four-shelf band art ] │ │
│ │  My Castle · Survival               │ │ Trials                  │ │
│ │  Played 2 hours ago      [ ▶ Play ] │ │ Little challenges…    › │ │
│ └─────────────────────────────────────┘ └─────────────────────────┘ │
│                                                                     │
│ [ Your worlds ] [ The Workshop ] [ Play together ] [ More to play ] │
│                                                                     │
│ v0.2.28 ★ Latest                                    (status line)   │
└─────────────────────────────────────────────────────────────────────┘
```

- **Top bar:** wordmark on the left. On the right:
  - **Profile chip.** Signed out: "Guest", opening the sign-in dialog. Signed in: short npub, with a small menu: "Your address" (the copyable npub and QR, as in the Friends column) and "Sign out".
  - **Settings.**
  - **Quit** (native) or **Exit** (web).
  - The guest line under the header goes. Its job moves into the chip's tooltip: "Sign in to play with friends." The old "save to the cloud" wording claimed more than the build does.
- **Continue (hero):**
  - Shows the most recently played world, with its picture (§6), name, mode and "Played …", plus a large **Play**. Play uses that world's start position (§4.2).
  - Clicking the card anywhere but Play opens that world's page in Your worlds.
  - No worlds yet: the hero becomes "Start your first world", with **Create a world**.
  - On web, while worlds are still loading, the hero shows "Loading your worlds…" instead of the empty state.
- **Trials tile:** the existing tile, enlarged to match the hero's height. It opens the gallery.
- **Four destination tiles,** equal size, each with an icon, a title and a one-line subtitle:
  - **Your worlds:** "Every world you've made." Subtitle shows how many worlds there are.
  - **The Workshop:** "Redesign blocks, mobs and your skin."
  - **Play together:** "Friends, invites and games nearby." On web this tile opens the existing desktop-app explainer (Join/Host) instead.
  - **More to play:** "Official games and things people share" (was "The Stash"; see §11).
- **Footer:**
  - The version and update state move here, bottom-left, unchanged in behaviour: badge, update link, Update now, progress, Restart.
  - The last import or export message sits on the right.
- **Layout by width.** Each step down keeps the same order and the same safe area:
  - Wide: hero and Trials side by side, then four tiles in one row.
  - Tablet: hero full width, Trials full width, then tiles in a 2×2 grid.
  - Phone: everything in one column.
  - The page scrolls as one, with a visible scrollbar.

## 4. Your worlds (page)

Header: `‹ Back`, "Your worlds", then on the right:
- **+ New world** (primary): the existing Create dialog.
- **Import ▾** (native):
  - "Open a world file…" (was Import world…)
  - "Bring in everything from the transfer folder" (was Import from folder)
  - "Bring in your worlds from the web game…" (was Import a web profile…)
- On web, instead: **Restore from a file** and **Save everything to a file** (was Export everything).

Status and handover lines sit under the header.

### 4.1 World grid
- Cards: a 16:9 picture (§6), name, mode badge, last played, size. The newest is first.
- The mode badge names the real mode: Survival, Creative, Adventure or Spectator. Adventure and Spectator used to read SURVIVAL.
- `PUBLISHED` / `EDITS PENDING` stay as a small badge.
- Trial arenas (`exp-*` folders) and the Workshop are **not** listed. They are reached from Trials and the Workshop tile.
- Selecting a card (click, or Enter on the focused card) opens the world's detail panel: on the right on wide screens, as a full page on narrow ones.

### 4.2 World detail
- A big picture, the name, the description and the mode, then the facts: size, last played, version.
- **▶ Play** (primary, large).
- **Start at:** the spawn choice, moved here from the global toolbar row, because it applies only to Play. It is still the same single remembered preference (`spawn_pref`), with the same 9 options.
- Secondary, native only:
  - **Host on this network** (was Host).
  - **Host for friends online** (was Host online). When signed out it stays visible but disabled, with its existing reason shown as a line of text under it (not only a tooltip).
- **More ⋯** menu:
  - "Rename and describe…" (Edit)
  - "Make a copy" (Fork; native only)
  - "Save a copy as a file…" (Export…; native, not Android)
  - "Save a copy to the transfer folder" (Export to folder; native, not Android)
  - "Download a backup" (Back up; web)
  - a divider, then **"Delete this world…"** (red), which opens the existing type-the-name confirmation.
- **Hidden** (rule 6): Host (it only played solo) and Fork (it always failed) on web; the export entries on Android (they only fail).
- **Stash:** the per-world toggle moves into More ⋯ as "Keep in your Stash: On / Off", with the same labels, colours and tooltips. The Stash dot leaves the world card and sits beside that entry. Whether the toggle should exist at all on native is an owner call: native has no visible upload behind it, and the dot never turns green.

## 5. Play together (page, native)

One page for everything about other people. It used to appear only on screens 1360 pt or wider, so phones and tablets had none of it. Header: `‹ Back`, "Play together". Sections, in a column on narrow screens and two columns on wide:

1. **Join a game:** the Join Game dialog's content inline: "Games on this network" and the room-code field.
2. **Friends:** the contact list with Join, **Add a friend**, and the Signet contacts row.
3. **Your address:** QR and copy (signed in), or the sign-in prompt (signed out).
4. **My servers** and **My Keeps:** unchanged rows.

Hosting stays on a world's page (§4.2), because you host a particular world. The Friends column and the Signet-unreadable strip leave the home page. The strip's warning and Disconnect move to the top of this page, and to the notice banner when a host is refused, as now. Red lines are unchanged: people you already know, no directory, no social framing.

## 6. World pictures

- **For now, every world card and the Continue hero show a placeholder.** It is a small isometric diorama drawn with the Trials diorama code, chosen by mode and world type (grass and oak for survival, quartz and glass for creative, sand for a desert seed), on that mode's colour band. It is never an empty grey box.
- **Follow-up (not in this build):** capture the world view on save-and-quit (the postcard capture path, 320×180) to `<world>/picture.png` and show it instead. The capture must never block or fail a save.

## 7. More to play (page; was "The Stash")

- Header: `‹ Back`, "More to play".
- **AxeNStax Official:** Hash Dash and Satori Rush, as large cards in the gallery style, each with Play. They still work offline.
- **Look up a creator** (web only): the address field (hint "Their address (npub1…)") and the results, with Play and Adopt skin. It is hidden on native, where Browse only printed "Live browse is a PWA feature".
- The side column leaves the home page.

## 8. The Workshop

- The tile enters the Workshop.
- Reset moves off the home page. It becomes "Clear the Workshop floor…" in a small ⋯ menu on the Workshop tile, with the same confirmation dialog and the same action.

## 9. Where every old control went

| Old | New |
|---|---|
| Wordmark, version line, update badge, Update now, Restart | Wordmark top-left; version and update in the footer |
| "Your Worlds" subtitle | Gone (it's a tile and a page title) |
| Settings, Sign in / Sign out, Quit / Exit | Top bar: Settings; profile chip (sign in, your address, sign out); Quit / Exit |
| Guest line, signed-in npub line | Profile chip and its tooltip |
| Stash sync line (dead) | Removed |
| Notice banner | Unchanged: top-centre, inside the safe area |
| Trials tile | Home, beside Continue |
| + Create New World | "+ New world" on Your worlds; "Create a world" in the empty hero |
| The Workshop / Reset | Workshop tile / its ⋯ "Clear the Workshop floor…" |
| Join Game | Play together → Join a game |
| Import world… / Import from folder / Import a web profile… | Your worlds → Import ▾ (renamed, §4) |
| Restore from file / Export everything (web) | Your worlds header (renamed, §4) |
| ☁ Stash / 🎮 Join / 🖧 Host explainers (web) | Play together tile opens the Join/Host explainer; the Stash explainer is no longer needed (the More to play page works on web) |
| Spawn dropdown | World detail → "Start at" |
| Status line, handover lines | Footer (status); Your worlds header (handover lines) |
| World card: Play, Host, Host online | World detail: Play; Host on this network; Host for friends online |
| World card: Edit, Fork, Export…, Export to folder, Back up, Delete | World detail → More ⋯ (renamed, §4.2) |
| World card: Stash toggle and dot | World detail → More ⋯ "Keep in your Stash" |
| Stash column (Browse, Official) | The More to play page (Browse becomes "Look up a creator", web only) |
| Friends column, Signet strip, Add a friend, My Servers, My Keeps | Play together page |
| Empty state ("No worlds yet") | Hero "Start your first world"; Your worlds shows the same message with "+ New world" |

## 10. Code shape

- New modules, each under about 600 lines:
  - `ui_safe_area.rs`: the helper, scale and tests.
  - `home_ui.rs`: home.
  - `worlds_ui.rs`: the world grid and world detail.
  - `play_together_ui.rs`: hosts the existing `friends_ui` / `lan_ui` / `signet_contacts::ui` / keeps pieces.
  - `stash_page_ui.rs` (the More to play page).
  - `world_picture.rs`: placeholder choice (capture comes later).
- `menu.rs` keeps `MenuAction`, `MenuState` and the dialogs; its old lobby body is deleted, not left beside the new one.
- Escape on any inner page (and in the gallery) goes back one page and never quits. The `lib.rs` lobby-Escape guard covers every `LobbyPage`.
- A `LobbyPage { Home, Worlds, PlayTogether, Stash }` field on `MenuState`. The Trials gallery keeps its own open flag.
- Every `MenuAction` the old lobby emitted is still reachable, except the hidden ones above.
- `--shot-lobby` gains:
  - `--page home|worlds|together|stash`
  - `--select <n>`, so world detail can be shown
  - a TV size of 1920×1080, alongside desktop, tablet and phone

## 11. Coming from the Keep work (not built here)

The owner expects "Stash" to give way to **your Keep** (Fray design, `keep-and-ingle.md`: "your Keep is where your stuff lives"; in a game "Host on my Keep", for backup "Back up to my Keep"). When that lands: the per-world "Keep in your Stash" entry in More ⋯ becomes the Keep backup or "promote to my Keep" action; "Host for friends online" may become "Host on my Keep"; My Keeps stays on Play together. Those labels live in one place (`lobby_words`) so the rename is one edit. To avoid a clash, the old "The Stash" side panel (official games and browsing a creator's open stash) is renamed **More to play** now. Component names (Fray, Ingle, Bothy, WildBloom Node) never appear in player UI.

As built (slice A): `lobby_words` is the `words` module at the top of `game/engine/src/worlds_ui.rs`.
