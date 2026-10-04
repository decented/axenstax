# Controls

Every key and button in Axe'n'Stax. Keyboard + mouse is the primary control scheme. Gamepad and touch are supported for the core actions (move, look, break, place, hotbar, inventory, pause, camera), but they do not have every keyboard action (see the notes under each section).

## Movement

| Input | Action |
|---|---|
| **W / A / S / D** | Walk forward / left / back / right |
| **Mouse** | Look (pitch + yaw) |
| **Space** | Jump |
| **Shift (Left Shift)** | Sneak — slower, doesn't fall off block edges |
| **Ctrl (Left Ctrl)** | Sprint while held |
| **Double-tap W** | Toggle sprint until you release W |
| **Double-tap Space** (Creative only) | Toggle flight |
| **Space (while flying)** | Fly up |
| **Shift (while flying)** | Fly down |

## Looking + targeting

The crosshair in the centre of the screen is what you're "pointing at". The block or mob under it is your **target**. Targeted blocks get a small white outline.

Mining/placing reach is about 5 blocks; attack reach (melee on mobs) is shorter, about 3 blocks. Anything beyond those you can't break, place, or hit.

## Mouse buttons

| Input | Action |
|---|---|
| **Left-click** | Break block / attack mob |
| **Left-click (hold)** | Continuously break — the block cracks and shatters |
| **Right-click** | Place block / use item / talk to villager / open crafting table / open campfire |
| **Right-click (food in hotbar + hungry)** | Eat |
| **Scroll wheel** | Cycle hotbar selection |

## Hotbar

| Input | Action |
|---|---|
| **1 – 9** | Select hotbar slot 1–9 |
| **Scroll up / down** | Previous / next slot |
| **Q** | Drop one of the currently held item (1.5 s pickup delay for you; other players grab it instantly) |
| **E** | Open inventory / crafting (and close it again) |
| **B** | Open **Inventory Explorer** — browse every block, tool, and material in the game with a search box. Creative: click any entry to add one to your inventory. Survival: hover to see how many you currently hold. Esc closes. |
| **J** | Open the **Challenge Board** — in-game challenges that nudge you to try specific features. |
| **Y** | Open the **Rig Studio** — author animated rigs in-game. Pick a skeleton (biped / quadruped / bird / fish / swaying plant), pick a **motion** (Walk, Idle, or Bounce), assign a block to each named part with the block in your hand, then **Spawn**. The rig stands in front of you and plays the motion; it saves with your world. If a part's block has a **micro-model** (a built-and-baked mini sculpture), the part is drawn as that sculpture, scaled to fit the limb, instead of a plain box. |
| **F5** | Cycle the **camera** — first-person → over-shoulder → orbit third-person → back. |
| **H** | Toggle the objective/help panel. If no Trial is active, this shows a controls cheat-sheet instead (T/chat, F5, J, N, the empty-hand pet-follow gesture, sneak-for-breeding). |
| **N** | Summon / dismiss Satoshi, the in-game guide. |
| **M** | Open the full-screen map (see **[Maps & Coordinates](maps-and-coords.md)**). |

## In the inventory / crafting screen

| Input | Action |
|---|---|
| **Left-click slot** | Pick up the stack (or place a held stack) |
| **Left-click again on another slot** | Drop the held stack into the new slot |
| **Hover** | Tooltip shows the item name (and stack count) |
| **Esc** | Close the inventory — your held cursor item drops back into your bag automatically |

When you're carrying an item on the cursor, you'll see a small floating icon + a black-pill label telling you what you're holding.

## Crafting table (right-click a crafting table block)

Same as the inventory, but the crafting grid is 3×3 instead of 2×2. Bigger recipes (tools, bread, the Village Bell) need a crafting table.

## Campfire (right-click a placed campfire)

Custom UI — see **[Food & Cooking](food-and-cooking.md)** for the slots and how it works.

## Chat / commands

| Input | Action |
|---|---|
| **T** | Open chat overlay (text input) |
| **/** | Open chat overlay with `/` pre-filled (for typing a command) |
| **Enter** (with chat open) | Submit |
| **Esc** (with chat open) | Cancel |
| **↑ / ↓** (with chat open) | Scroll through your command history |

See **[Chat & Commands](chat-and-commands.md)** for the full command list.

## Modes & menus

| Input | Action |
|---|---|
| **Esc** | Pause / close UI / back |
| **F3** | Debug overlay (coords, biome, performance) |
| **F7** | Toggle the spawn-proof overlay (debug) |
| **F11** | Toggle borderless fullscreen |

## Cinematic Camera — the Director (PC only)

A built-in freecam + film studio for capturing footage. **F6** detaches the camera
and freezes your body; **F6** again returns you. Full detail: **[Cinematic Camera](cinematic-camera.md)**.

| Input | Action |
|---|---|
| **F6** | Enter / exit the Director (body freezes while active) |
| **W A S D** | Fly (relative to look) · **Space** up · **Left Shift** down · **Left Ctrl** boost |
| **F9** | Cycle mode — free-fly → path → tripod → follow → look-at → POV |
| **F8** | Hide / show the HUD + crosshair (clean frame) |
| **F10** | Cycle the follow / look-at / POV target |
| **Enter** | Drop a keyframe at the current camera pose |
| **F12** | Play / stop the keyframe (dolly) path |
| **Backspace** | Clear the keyframe path |
| **F4** | Start / stop recording the session to an `.axereplay` file |

## Gamepad (controller)

Both Xbox-style and PlayStation-style controllers work. Up to 4 controllers for couch co-op split-screen.

| Input | Action |
|---|---|
| **Left stick** | Move |
| **Right stick** | Look |
| **A / X (cross)** | Jump |
| **B / Circle** | Inventory toggle |
| **X / Square (West)** | Cycle the third-person camera (gamepad equivalent of F5) |
| **Y / Triangle** | Debug overlay |
| **Right trigger** | Break / attack |
| **Left trigger** | Place / use |
| **LB / L1** | Hotbar previous |
| **RB / R1** | Hotbar next |
| **D-pad Down** | Drop one item |
| **Left stick click (L3)** | Toggle sprint |
| **Right stick click (R3)** | Toggle sneak |
| **Start** | Pause |
| **A double-tap** | Toggle flight (Creative) |

**Not on the gamepad yet:** the Map, Challenge Board (J), Satoshi (N), chat, block-ghost rotate, Inventory Explorer and the Workshop editing keys. Use a keyboard for those.

## Touch (mobile / tablet web)

A fixed on-screen layout, not gesture-based:

- **Bottom-left** — a floating joystick for movement.
- **Bottom-right** — a 2×2 action cluster: **Break** / **Place** on the top row, **Sneak** / **Jump** on the bottom row. These are discrete tap buttons, not tap-to-break or long-press gestures.
- **Bottom-centre** — the hotbar + an **Inventory** button.
- **Top-left** — **Pause** and **Chat** buttons.
- **Top-right** — **View** (cycles perspective, same as F5) and **Zoom** buttons.

**Not on touch yet:** drop item, block-ghost rotate, Inventory Explorer, the Workshop editing keys, shift-click, Map, Challenge Board (J) and Satoshi (N).

## Tips

- If your hands fall onto the hotbar number keys by accident a lot, try selecting your most-used tool slot and just leaving it there. The scroll wheel is faster to change holding-item.
- **Q** drops one item — if your inventory is full and you really don't want one stack of cobblestone, Q-spamming clears it fast.
- **B** opens the **Inventory Explorer** — type a few letters and the list filters as you type. Useful when you're trying to remember what something is called or what you've collected so far.
- **Esc** is the universal "back". When in doubt, press Esc.
