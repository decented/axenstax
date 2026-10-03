# Lobby Redesign — design

**Date:** 2026-06-02
**Status:** APPROVED IN PRINCIPLE (Staxolottle 2026-06-02 — three decisions locked
below); spec for review before the rewrite.
**Branch:** `feat/axenstax-consume-stash`
**Scope owner term:** "lobby" = the post-sign-in **world-list / main menu**
(`menu.rs::draw_main_menu` / `draw_world_card`), not the sign-in page. See
[[feedback_lobby_means_world_list]].

---

## Why

Playtest of the Stash work exposed that the lobby itself is broken. Headless
screenshots (`--shot-lobby`, three sizes) confirm:

- **Not responsive.** Cards use fixed widths in a wrapped horizontal grid; at
  1280 the 2nd column + "Create New World"/"Join Game" run off the right edge,
  and at 390 (phone) card content overflows far past the viewport. Unusable on
  anything but one width.
- **No way out.** No back / sign-out — you can't return to the entrance.
- **Card interaction is undiscoverable.** The whole card isn't clickable; only
  the game-mode badge reliably selects it (the card uses a non-interactive frame
  + a global pointer-click read on hover). Users think it's broken.
- **Stash control is cramped/cut off** ("Stash:" with the state truncated).

## Locked decisions (owner, 2026-06-02)

1. **Card interaction:** whole card is tappable → selects + expands its actions
   inline. Double-tap / Enter = Play.
2. **Scope:** world-list only this pass (header + back/sign-out, tappable cards,
   responsive layout, working Stash traffic-light). Leave a visible slot for a
   future social/presence strip but don't build it.
3. **Verify:** headless `--shot-lobby` screenshots (built, commit `d5cbd65`).

## Design

### Layout (responsive, two-region) — owner decision 2026-06-02
- **Wide (≥ ~900 logical px): two regions.** Left = the worlds area (a
  **responsive multi-column grid**: card min width ~320px → columns =
  `floor(area_width / ~340)`, clamped 1–3, cards sized to fill evenly). Right =
  a fixed ~300px **side panel** reserved for social/presence ("Who's online")
  and other relevant info — a styled placeholder this pass, wired later. Use an
  `egui::SidePanel::right` shown only when wide.
- **Narrow (< ~900): single column**, worlds only, full-width cards in a
  `ScrollArea`; the social panel is hidden (it returns when there's room). This
  keeps the kid-friendly vertical scroll on phones/tablets (per the 2026-05-21
  note) while using the extra width on desktop.
- Card internals **stack/wrap**: title + badges on row 1; description on row 2;
  meta (last played · size) on row 3 — never a single overflowing row.
- Min touch target 40px; generous padding. Worlds area always scrolls.

### Card states
- **Collapsed:** name, mode badge, version, Stash light, one-line description,
  meta. Entire card is one click-surface (egui `Sense::click()` over the rect).
- **Selected/expanded:** action row appears inline — **▶ Play** (primary,
  prominent) · Host · Edit · Fork · Back up (WASM) · **Stash light** · Delete
  (right-aligned, danger styling). Buttons wrap on narrow widths.
- Double-click / Enter on a card = Play.

### Header (new)
- Top bar: title/wordmark, the persona (npub, per [[feedback_npub_only_display]])
  when available, and a back control: **web → "Leave" / "Sign out"** returns to
  the entrance (sign-in page); **native → "Quit"** exits the app (native has no
  entrance — owner left the choice to me, 2026-06-02).
- Primary **+ Create New World** stays prominent (top of the worlds area).
- The **"Who's online" / social** placeholder lives in the right side panel
  (wide screens), not a separate slot.

### Stash traffic light (fix + fit)
- Keep 🔴 off / 🟠 on-unconfirmed / 🟢 on-stashed, but give it room so the label
  isn't truncated, and show it in both collapsed (compact dot) and expanded
  (dot + label) forms.
- **Fix green for real:** the cloud manifest is keyed by `display_name` but the
  traffic light matches on the sanitised `folder_name`, so green never hits.
  Make them consistent — key the cloud entry by a stable id (folder name) and
  carry display name in the entry — so a stashed world actually resolves green.
  (Tracked separately from the visual work; both land together.)
- When no capable signer, show the amber state with a "Sign in for Stash" hint
  so it's explained, not mysterious.

### Interaction plumbing
- Replace the frame-hover + global-pointer hack with a proper
  `ui.interact(rect, id, Sense::click())` per card; derive select/expand/play
  from its `.clicked()` / `.double_clicked()`. Buttons inside still win their own
  clicks (check button responses first).

## Non-goals (this pass)
- Multi-column grid, world thumbnails/art, social/presence wiring, world search,
  drag-reorder. Slots/space left where sensible.

## Testing / verification
- `--shot-lobby` at desktop/tablet/phone after each iteration; eyeball that
  nothing overflows, cards expand, header + back present, traffic light legible.
- `check.sh` green (clippy/build/test/wasm/bundle).
- Add `sample_menu_state` variants if needed to show all three traffic-light
  colours in a shot.
- Browser playtest (owner): real responsiveness on actual devices + that the
  whole card taps.

## Follow-on
- Social/presence strip (its own spec). Multi-column + thumbnails when worlds
  carry preview art. The Stash "out-of-sync" (amber-when-stale) detection from
  the Stash Layer-2 spec.
