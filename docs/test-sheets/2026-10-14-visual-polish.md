# Test sheet — Copperline visual polish (2026-10-14)

**Combined source checkpoint:** `818e2691507047508aed1e5c60bd15a8d3cfc376`. Engine v0.2.27,
wire 91, WorldSave 53 fields, save-layout revision 2 and canonical participant
save v3. This sheet is a documentation-only follow-up to that source checkpoint.
The final regression report records the exact documentation-inclusive tested
commit/tree and native executable SHA. Use that exact build for the manual
checks; final combined acceptance remains pending until its gate and review pass.
Mirror remains the default and Strict remains partial/parked.

**Scoped visual source:** `e59b5e36` (tree `941183bf`), including the lobby
pass from `23b6f23c`, normally integrated into the combined checkpoint. No
deployment is authorised. Record the actual tested commit/platform below.

**Automated visual evidence:** six native headless lobby renders at 1280×800,
834×1112 and 390×844, with populated and empty world lists. Both source and
independent render review passed. Five final in-world noon/midnight/narrow
world/HUD renders and six third-person camera renders also passed scoped review.
GPU5, HUD18, touch10, camera59 and shader1 targeted checks are green; Clippy and
the native build are green. Exact source, executable and capture provenance is
under `logs/visual-polish/in-world/after-depth-lighting/` and
`final-target-evidence.json`; review is `handoff/in-world-presentation-review.md`.
These renders and automated checks do not prove live pointer, keyboard,
controller, touch or in-world hotbar behaviour, or replace the integrated gate.

**Needs:** native desktop; a narrow window or phone/tablet build for the narrow
layout. Keep an existing world available. Mark each line ✓ / ✗ / felt off.

## Lobby

- [ ] The Axe’n’Stax wordmark is sharp and readable at desktop and narrow widths.
- [ ] World names, descriptions and last-played/size metadata are readable.
- [ ] The selected world has a clear copper outline; selection remains clear
      without relying on the outline colour alone.
- [ ] At a narrow width, the right-hand edge of each card and every action stays
      on screen. Scroll to the last world and select it.
- [ ] Spawn appears on its own row. Open its menu, choose another preference,
      close and reopen it: the chosen label and available options remain clear.
- [ ] Settings, Sign in and Quit remain accessible beside the header. Open and
      close Settings, then open and cancel Create New World and Import.
- [ ] With an empty list, the mark, “No worlds yet” and Create New World action
      are clear; create a world and return to the lobby.
- [ ] Keyboard/controller focus is visible on buttons, combo boxes and fields;
      confirm an action activates only once.
- [ ] On a touch build, all actions remain tappable and the list scrolls normally.

## In a world

**Reviewed in-world candidate:** `e59b5e36` adds the atmospheric sky, survival
silhouettes, readable count/name badges, opaque foliage depth protection and
correct daytime foliage lighting. Actual narrow text bounds, cutout holes,
partial glass, front water and day/night/torch lighting have regression evidence.
The final combined gate and the following live acceptance steps remain pending.
Record the actual tested commit and platform in the manual fields below.

- [ ] Look up/down and turn through a full circle at noon and midnight. The sky
      grades smoothly overhead and its horizon stays in the world as you look.
- [ ] Check the horizon and distant hills with fog on; change render distance,
      fog and brightness settings. No bright seam, flash or colour jump appears.
- [ ] Look outside in rain and during a lightning flash. Enter water, surface
      and step into a torch-lit cave: existing visibility and fog still behave.
- [ ] Look through leaf cutout holes at water, and at solid leaves in front of
      water. Leaves remain opaque where drawn; holes and glass reveal what is
      behind them. Check a tree at noon, midnight and beside a torch.
- [ ] Switch first-person/over-shoulder/orbit cameras. The sky remains stable
      and the avatar/held item keeps its normal daytime/night-time lighting.
- [ ] With full, half and depleted health/hunger, hearts and drumsticks remain
      recognisable against light terrain and a dark cave. Check damage/eating.
- [ ] Read two-digit counts over bright and dark block textures. The held-item
      name is centred and the free-slot/arrow label stays inside the right edge.
- [ ] Cycle all nine hotbar slots against light sky, grass, stone and a dark cave.
      The selected slot’s gold outline and item remain clear in each setting.
- [ ] Health, hunger, item counts and the crosshair remain readable.
- [ ] Open inventory, crafting and the pause menu; focus, selections and text
      remain readable, then return to play normally.
- [ ] Repeat with split-screen if available: each seat keeps its own selection.

## Notes

Tested commit / platform / display scale:

Any text too faint, action hard to find, or layout that clips:

This visual sheet does not authorise a Strict or single-player simulation default
flip. Their separate owner playtest gates remain pending.
