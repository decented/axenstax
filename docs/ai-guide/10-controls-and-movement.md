<!-- SOURCE: game/engine/src/{input.rs, player_intent.rs, physics.rs, camera.rs, touch_input.rs, gamepad.rs}, plus game_loop.rs + main.rs for the chat/zoom/free-look/Escape/F-key bindings | Verified against code 2026-06-22 -->

# 10 — Controls & Movement

**Purpose:** the exact, verified key/button bindings and movement physics, so the assistant can tell a player precisely how to move, look, build and mine — and never instruct an action that isn't bound on their device.

> **Honesty rule for this page:** keyboard + mouse has every binding. **Touch and gamepad cover core play but leave several actions unbound** — those are listed at the bottom. Never tell a touch or gamepad player to use an action this page marks unbound on their device.

---

## 1. Keyboard & mouse (the full set)

All bindings below are read directly from `input.rs` (which maps physical keys to a `PlayerIntent`) plus the window event loop in `main.rs`/`game_loop.rs` for the keys handled there. UK keyboard QWERTY assumed.

### Movement & camera (held)
| Key | Action |
|-----|--------|
| **W / A / S / D** | Move forward / left / back / right |
| **Space** | Jump (hold) — also swim up in water, fly up in flight |
| **Left Shift** | Sneak (slow, careful walk) — also dive down in water, fly down in flight |
| **Left Ctrl** | Sprint (run faster) — hold while moving |
| **Double-tap W** | Start sprinting (alternative to Ctrl; sprint ends when W is released) |
| **Mouse move** | Look around (turn the camera) |

### Build / mine / interact
| Input | Action |
|-------|--------|
| **Left mouse button** | Break / mine the block you're looking at (hold to keep mining); also attack |
| **Right mouse button** | Place a block / use the held item / interact |
| **Mouse scroll wheel** | Cycle the selected hotbar slot |
| **1–9 (number keys)** | Select hotbar slot 1 to 9 directly |
| **Q** | Drop one of the held item |
| **E** | Open / close the inventory & crafting screen |

> **Context overload (verified):** while a build "ghost"/blueprint preview is active, **Q rotates the ghost 90° anticlockwise** and **E rotates it 90° clockwise** instead of dropping/opening inventory. The game decides which meaning applies by context — outside a ghost they are plain Drop / Inventory.

### Camera & view
| Key | Action |
|-----|--------|
| **F5** | Cycle the camera perspective: first-person → over-the-shoulder → behind (orbit) → back to first-person |
| **C** (hold) | Hold-to-zoom (spyglass-style narrowed view). Keyboard only; releases when you let go or open a menu |
| **Left Alt** (hold) | Free-look — look around in third-person *without* changing where you're aiming. **Only works if the "third-person free-look" setting is on (it is off by default) and you're already in a third-person view.** Don't promise it unless the player has enabled it |

### Screens, overlays & system
| Key | Action |
|-----|--------|
| **T** | Open the chat / command line (type a message or a slash command) |
| **/** | Open the chat line pre-filled with `/` (ready for a slash command) |
| **B** | Open / close the Inventory Explorer overlay |
| **M** | Open / close the full-screen map (outside the Workshop) |
| **J** | Open / close the challenge board |
| **Y** | Open / close the Rig Studio |
| **Escape** | Back / pause: pauses the game while playing, closes an open crafting or villager screen first, resumes when paused, backs out of menus |
| **F3** | Toggle the debug overlay (position / info) |
| **F7** | Toggle the spawn-proof overlay (shows where mobs can spawn at night) |
| **F11** | Toggle fullscreen (**native build only** — in the browser the browser owns fullscreen) |

### Workshop editor keys (only meaningful inside the Workshop)
These are bound on **keyboard only**. They do nothing useful outside the Workshop editor.
| Key | Action |
|-----|--------|
| **G** | Eyedropper — grab the colour under the crosshair |
| **M** | Cycle the edit-symmetry mode (inside the Workshop; M is the map key elsewhere) |
| **V** | Toggle Paint / Sculpt mode (Paint is the safe default — you must press V to start carving) |
| **P** | Pin the locked working copy |
| **K** | Open / close the Wardrobe panel |

---

## 2. Movement & physics (what the body actually does)

All numbers below are the literal constants in `physics.rs`. Speeds are in **blocks per second**; the simulation runs at **20 ticks per second**.

### Speeds
| State | Speed (blocks/sec) | Notes |
|-------|--------------------|-------|
| Walk | **4.317** | Default ground speed |
| Sprint | **5.612** | Hold Ctrl / double-tap W while moving |
| Sneak | **1.295** | Hold Left Shift — slow and careful |
| Swim | **2.5** | In water |
| Swim (sprint) | **4.0** | In water, with sprint held |
| Fly | **10.89** | Creative/Spectator flight |
| Fly (sprint) | **21.78** | Flight with sprint held |

### Jumping & falling
- A jump is an upward impulse and **only fires when you're on the ground**.
- **Sprint-jumping** (jumping while sprinting and moving) adds a small forward boost, so you cover more distance.
- Gravity pulls you down each tick; falling speed is **capped** (terminal velocity) so you can't fall fast enough to clip through a thin floor.
- There is no separate fall-damage description verified on this page — do not claim a specific fall-damage rule here.

### Sneaking (the careful walk)
- Sneak is slow.
- **Sneak edge-protection (verified):** while sneaking on the ground you **won't walk off the edge** of a block — your movement is stopped at the lip. This makes building out over a drop safe. You *can* still sneak-jump off deliberately, and you can slide along an edge.

### Swimming
- In water you move freely in all directions (like a gentle fly): **Space swims up, Left Shift dives down.**
- With no up/down input you slowly sink.

### Climbing ladders (verified)
- While your feet are on a **ladder**, gravity is cancelled and you cling in place.
- **Hold Space to climb up**, **hold Left Shift to climb down**, ~2.5 blocks/sec. Let go of both and you hold position.

### Flight (Creative only)
- **Double-tap Space** toggles flight on/off — but **only in Creative mode.**
- In Survival and Adventure modes flight is forced off (you're grounded). Switching out of Creative drops you.
- **Spectator mode** always flies and passes straight through blocks (noclip). Creative flight still collides with blocks — you can't fly through walls in Creative.

### Player body (fixed, for reference)
- Hitbox: **0.6 wide × 1.8 tall**; eye height **1.62**. Auto-step height **0.5** (you step up half-block ledges without jumping). These are the player's only hitbox and never change with skins/cosmetics.

---

## 3. Camera (first- vs third-person)

- The default view is **first-person** (the eye is the camera; you see your held item/hand).
- **F5 cycles** through three perspectives in a loop: **first-person → over-the-shoulder → behind/orbit → first-person.**
- Crucially: **aiming always comes from your true eyes, in every camera mode.** Pulling the camera back to third-person changes only what you *see* — mining, placing, and combat still aim from where your character is actually looking.
- **Free-look** (Left Alt, third-person only) lets you swing the camera around without re-aiming — but it's an **opt-in setting that's off by default**, so treat normal play as "camera follows where you aim".

---

## 4. Touch controls (phone / tablet / touchscreen Chromebook)

The on-screen overlay (`touch_input.rs`) appears automatically on a touch device. Layout:

| Area | Control |
|------|---------|
| Bottom-left | Floating virtual **joystick** — drag to move; **push to the edge to sprint** |
| Bottom-right (2×2 cluster) | **Jump**, **Sneak**, **Break** (mine), **Place** |
| Bottom-centre | The 9-slot **hotbar** (tap a slot to select) + an **Inventory** button at its right end |
| Anywhere else on screen | Drag to **look** (rotate the camera) — a second finger looks while the joystick thumb moves |
| Top-left | **Pause** and **Chat** buttons |
| Top-right | **Perspective** (camera cycle) and **Zoom** (hold) buttons |

So on touch you **can**: move, sprint, look, jump, sneak, break, place, select hotbar slots, open inventory, pause, open chat, cycle the camera, and zoom.

### ❌ Not available on touch (verified `false` in `touch_input.rs::to_intent`)
These have **no on-screen button**, so a touch player cannot currently do them — route around them:
- **Drop item** (the keyboard's Q)
- **Rotate a build ghost / blueprint** (Q/E on keyboard)
- **Open the Inventory Explorer** (B on keyboard)
- **Workshop editor tools:** eyedropper, cycle symmetry, pin, Wardrobe, Paint/Sculpt toggle

---

## 5. Gamepad / controller (Xbox-style mapping)

From `gamepad.rs` (`state_to_intent`). Works on native and in the browser, USB or Bluetooth.

| Control | Action |
|---------|--------|
| **Left stick** | Move |
| **Right stick** | Look |
| **A (South)** | Jump (hold to keep jumping) |
| **Double-tap A** | Toggle flight (Creative) |
| **B (East)** | Open / close inventory |
| **X (West)** | Cycle camera perspective |
| **Y (North)** | Toggle the debug overlay |
| **Right Trigger (RT)** | Break / mine / attack |
| **Left Trigger (LT)** | Place / use |
| **Left bumper (LB) / Right bumper (RB)** | Cycle hotbar slot (prev / next) |
| **Left stick click (L3)** | Toggle sprint on/off |
| **Right stick click (R3)** | Toggle sneak on/off |
| **D-pad Down** | Drop one item |
| **Start** | Pause |
| **D-pad (in menus)** | Navigate menus |

So on gamepad you **can**: move, look, jump, fly (Creative), sneak, sprint, break, place, cycle hotbar with the bumpers, open inventory, drop, pause, toggle the camera and debug overlay, and navigate menus.

### ❌ Not available on gamepad (verified `false`/`None` in `gamepad.rs::state_to_intent`)
- **Direct hotbar number-select** — there's no "jump to slot N"; you must cycle with the bumpers (the comment marks direct D-pad hotbar selection as a later phase).
- **Rotate a build ghost / blueprint**
- **Open the Inventory Explorer**
- **Workshop editor tools:** eyedropper, cycle symmetry, pin, Wardrobe, Paint/Sculpt toggle

---

## Deferred / not yet
- **Touch and gamepad parity is incomplete.** Each is missing several actions listed above (drop on touch; direct hotbar-select on gamepad; the build-ghost rotate, Inventory Explorer, and all Workshop editor tools on both). A player who needs those should use a keyboard. (Cross-checked against `99-accuracy-and-deferred.md`, which flags ~7 unbound actions per device.)
- **Third-person free-look (Alt)** is an opt-in setting, off by default — don't present it as a standard control.
- This page does **not** assert a fall-damage rule, a specific in-water breath/drowning rule, or any control not listed above — if asked, say it's not something this page can confirm.
