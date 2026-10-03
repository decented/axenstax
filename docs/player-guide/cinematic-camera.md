# Cinematic Camera (the Director)

A built-in **freecam + film studio** for capturing beautiful footage of your world — flythroughs, build tours, orbiting hero shots, slow reveals. It's the equivalent of Minecraft's Freecam / Replay mods, but built straight into the engine instead of bolted on, so the camera is *just a coordinate*: no hitbox, no collision, no jank.

> **PC desktop only, for now.** The Director is in the **native desktop build**, not the browser/PWA version. It's **alpha** — the keys below are placeholders and may change as we tune the feel. Tell Staxolottle what works and what doesn't.

## What it does

Press **F6**. Your body **freezes** where it stands and the camera detaches — now *you* fly the camera. Breaking and placing blocks are switched off while you film (so you can't mine by accident), and the mouse keeps controlling the look. Press **F6** again to drop back into your body and carry on playing exactly where you left off.

While the Director is active you're flying one cinematic camera that has six **modes**. Cycle them with **F9**.

## Flying the camera

| Input | Action |
|---|---|
| **W / A / S / D** | Move (relative to where you're looking) |
| **Mouse** | Look around |
| **Space** | Rise straight up |
| **Left Shift** | Drop straight down |
| **Left Ctrl** (hold) | Boost — fly faster |
| **F8** | Hide / show the HUD **and the crosshair** — for a clean frame |

There's no collision: the camera flies straight through blocks. (It can only see chunks that are already loaded around your body — fly far past the edge of the loaded world and you'll see empty space. This is deliberate: it means freecam can never be used to scout or X-ray.)

## The six modes — F9 cycles through them

| Mode | What it's for |
|---|---|
| **Free-fly** | Steer by hand — the classic freecam. Best for flythroughs and stills. |
| **Path** | Play back a smooth **keyframed dolly** (see below) — a hands-free camera move. |
| **Tripod** | Lock the camera in place and just aim — a fixed wide shot while the action plays out. |
| **Follow** | Sit at a fixed offset behind a **target** and chase it; you still aim freely. |
| **Look-at** | **Orbit** a target with the aim auto-locked onto it. **W / S** change the orbit distance. |
| **POV** | See through a target's eyes — their exact position and head angle. |

**Targets** (for Follow / Look-at / POV) are chosen with **F10**, which cycles through the available players. In single-player the only target is **your own frozen avatar** — so you can orbit your build with yourself in the shot, or drop into the point of view of wherever you were standing. *(Filming other players in multiplayer is coming.)*

## Building a dolly shot (a keyframe path)

A **path** is a smooth camera move you set up once and play back hands-free:

1. Fly to where you want the shot to **start**. Press **Enter** to drop a keyframe (it records the camera's exact position + angle).
2. Fly to the next point. Press **Enter** again. Repeat for as many points as you want.
3. Press **F12** to **play** the move — the camera glides smoothly through every keyframe in order (a curved path that eases through the points, not robotic straight lines). Press **F12** again to stop.
4. **Backspace** clears the path so you can start fresh.

Pair it with **F8** (hide HUD) and you've got a clean, hands-free flythrough ready to record.

## Recording a clip

| Input | Action |
|---|---|
| **F4** | Start / stop recording the session to a `.axereplay` file |

Press **F4** and a toast says `● Recording replay — F4 to stop`. Press it again and it saves,
e.g. `⏹ Replay saved: profile/replays/MyWorld-42.axereplay (128 frames)` — the filename is
**auto-generated** (your world's name plus a running number), not something you type, and a
very long recording may add a **"capped"** note if it hit the length limit.

> **How to get footage today:** the in-game **playback** for these recordings (re-flying the Director *through* a recording, with slow-motion and scrubbing) is **coming next** — right now an `.axereplay` is saved for that future feature. To capture video **now**, fly the Director with the HUD hidden (**F8**) and **screen-record it with OBS** (or any screen-capture tool). That's the intended alpha workflow — clean, smooth, and nothing can break it.

## All the keys at a glance

| Key | Action |
|---|---|
| **F6** | Enter / exit the Director (your body freezes while active) |
| **W A S D** | Move · **Space** up · **Left Shift** down · **Left Ctrl** boost · **mouse** looks |
| **F9** | Cycle mode — free-fly → path → tripod → follow → look-at → POV |
| **F8** | Hide / show the HUD + crosshair |
| **F10** | Cycle the follow / look-at / POV target |
| **Enter** | Drop a keyframe at the current camera pose |
| **F12** | Play / stop the keyframe path |
| **Backspace** | Clear the keyframe path |
| **F4** | Start / stop recording to `.axereplay` |

## Want a walkthrough?

There's a step-by-step tutorial in the Journey — **[Film Your World](https://learn.axenstax.com/docs/journey/learn-journey/film-a-cinematic-shot.md)** — that takes you through your first orbit and your first gliding flythrough.

## Coming next

- **In-game replay playback** — load an `.axereplay` and fly the Director *through the recording*: scrub the timeline, slow-motion, bullet-time. The recordings you make now are saved for this.
- **Filming other players** in multiplayer — Follow / POV on anyone, plus live spectator-cameras (a friend films you while you keep playing).
- An on-screen panel showing the current mode, target and fly-speed.
- Feel tuning — fly-speed, field-of-view, path easing. Your feedback shapes it.

---

*Axe'n'Stax is in alpha; if a key here does something different in your build, the game wins — and please tell Staxolottle so we can fix the page.*
