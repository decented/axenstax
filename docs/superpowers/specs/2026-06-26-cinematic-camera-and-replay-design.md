# Cinematic Camera & Replay — design

**Date:** 2026-06-26
**Status:** Design approved (owner + Axolittle). No code until a build phase is green-lit.
**Codename:** TBD ("Reel" / "Cinema" / "Director" — open item).

## Why

The near-term opportunity is **getting AxeNStax in front of YouTubers / content creators**. To make that real we need first-class tools for capturing good footage — the equivalent of Minecraft's Freecam / Replay Mod, but built in rather than bolted on. Beyond outreach, a downloadable, re-directable multiplayer replay is **content-engine fuel**: highlight reels, tournament VODs, "watch the match from any angle" — which feeds straight into the Showcase / Challenge-Engine UGC direction. Axolittle's framing: *"every multiplayer game is recordable footage anyone can re-cut."*

### Why this is a differentiator (not a me-too)

- **Clean, not hacky.** MC freecam mods fight a closed engine — frozen movement packets, fake camera entities, physics workarounds. We own the renderer, so the camera is just a coordinate (the modern [WI Freecam](https://www.curseforge.com/minecraft/mc-mods/wi-freecam) approach): no hitbox, no collision, no jank.
- **Safe in multiplayer**, where MC must ban freecam. MC servers run anti-freecam/anti-X-ray because a detached camera scouts bases/caves/ores. AxeNStax already solves this **architecturally**: chunks stream around the player **body**, not the camera (`chunk_stream.rs` keys on `slot.player.pos`), and buried ore is replaced with stone in the client stream. So our freecam physically can't scout beyond already-loaded chunks and sees no hidden ore. We can offer it everywhere. *(The one exception — networked spectator-cameras — is handled explicitly below.)*

## What already exists (the foundation)

The render camera is **already decoupled** from the body for third-person, so ~70% of the plumbing is in place:

- `Camera` separates `position` (true eye) from `render_eye()` (`camera.rs`) — third-person (F5: first-person / over-shoulder / orbit-behind) already renders from an offset with collision clamping.
- `apply_free_look()` already decouples *look direction* from *aim* (render-only) with auto-recenter.
- Spectator mode already flies with noclip — `play_mode.rs` + `physics.rs::tick_flying` — it just currently moves the *body*.
- The **only** tie between camera and body is one line per tick: `slot.camera.position = slot.player.eye_pos()` (`game_loop.rs:~2984`).
- Hold-to-zoom FOV exists (#44).
- The networking layer already broadcasts each player's **position + yaw + pitch** every tick (`PlayerState` in `protocol.rs:256`, used for avatar head-turning) — exactly the data follow-cam and first-person-POV need. And the `StateUpdate` delta machinery (`diff_entities`, `ProtocolId`, spawn/update/despawn) is, in effect, already a replay stream.
- Spec 04 §8 already designs a **three-tier spectator protocol** (Close / Standard / Mass, `SpectatorSnapshot`, a fan-out relay) — deferred from the LAN-coop MVP, but the wire design exists.

What's missing: camera↔body detachment for free-fly, keyframe paths, a hide-HUD toggle, the recorder/format, the Director (playback) UI, and standing up the networked spectator tier.

## Architecture — one camera, three drivers, plus recorders

The mental model: **one Cinematic Camera with a set of modes**, controlled by one of **three drivers**, fed (for replay) by **recorders**.

```
            ┌──────────── Cinematic Camera (the modes) ────────────┐
Driver A:   │  your own camera (live, local)                       │
Driver B:   │  a friend's spectator-camera (live, networked)       │
Driver C:   │  the Director (offline, over a recording)            │
            └──────────────────────────────────────────────────────┘
                                  ▲
                          Recorders feed C
                  (SP local-sim / MP client-stream / MP server full-session)
```

Layered deliberately: **Replay = Recording + the same Cinematic Camera applied to recorded time.** Build the camera first; every driver reuses it; recorders feed the Director.

### The Cinematic Camera (the modes)

One subsystem with a set of modes, reused by every driver. The freecam is just the camera in free-fly mode. **Each mode works live *and* in replay:**

- **Free-fly** — detach the camera and steer it manually (WASD + mouse, Space/Ctrl for up/down, Shift to boost; reuses the `tick_flying` math applied to a detached camera coordinate).
- **Path / dolly** — keyframed smooth move (position + orientation + time), Catmull-Rom/Bézier with ease-in/out → dolly / orbit / flythrough. (Authoring detail in the Director section.)
- **Tripod (fixed)** — lock the camera at a vantage point and let the action play out in frame (a static wide shot). Save/recall tripods on number keys.
- **Follow-cam** — track a target entity at a smoothed offset (chase a player as they move).
- **Look-at / orbit** — *you* drive position (free-fly or a path), but aim auto-locks to a chosen target — orbit or track a player without hand-animating the aim.
- **First-person POV** — render from a chosen player's eye + yaw + pitch ("see through their eyes"); the wire already carries this.

**Auxiliary** (any mode): hide-HUD (clean frame), zoom FOV (#44), roll/dutch tilt, speed control (two-clock model — see Director).

Design rules:
- **Render-only** → safe in every mode incl. multiplayer (anti-X-ray holds). *Caveat for the live-spectator driver — see Driver B.*
- **Block interaction gated off** while the camera is detached (raycast origin would be wrong; never an exploit).
- **Body frozen while *you* film your own camera (Driver A).** Single-player: fine. Live multiplayer: your body stands there, vulnerable, and the scene keeps moving — you can't re-time it (that's what Replay is for, and what a *friend* spectator-camera avoids, since your body keeps playing). Show a "● filming" indicator to others.
- Only the **single line** `camera.position = eye_pos()` becomes conditional; everything else (render-eye, free-look, fly math) is reused.

### Driver A — your own camera (live, local)

You fly your own camera in your session; your body freezes. Best for solo build tours / world showcases (SP), quick looks, and stills. Anti-X-ray safe (it only flies through chunks already loaded around your body); block-interaction gated.

### Driver B — a friend's spectator-camera (live, networked)

A friend joins your server as a **Close Spectator** (Spec 04 §8 — the richest tier: full entity state + partial chunks, free-flying) and drives the camera *live while you keep playing*. Best for live filming/casting, a friend shooting you, and **multi-cam** (several friends = several simultaneous angles).

- **The catch (unique to live spectators):** a networked spectator pulls *fresh* chunks around *its* camera, so the server streams it new areas — a **live-scouting vector** that Driver A and the Director don't have (a "friend" could relay base/ore/enemy positions mid-game). Mitigations, all applied:
  - **Same obfuscated stream as players** (buried ore → stone) — preserves the ore-X-ray protection.
  - **Operator-gated, trust-based** — default **off**; "friends-only by invite" (ties into the Axis-1 access model); not opened to randoms on a competitive server.
  - **Optional broadcast delay** — competitive worlds show spectators the game N seconds behind (standard esports practice) so live intel can't be relayed.
- **Server load (honest):** each Close Spectator costs roughly **one player's bandwidth** (~20 KB/s) but **less CPU than a player** — the server does no input processing or simulation for spectators (Spec 04 §8.4). World-sim cost is unchanged. So a handful of friend-cameras ≈ a couple of extra player slots, lighter. The spiky part is chunk streaming to a *fast* camera → **cap camera speed + chunk rate** (and Close already gets only partial chunks). Large *audiences* (not operators) use the cheaper Standard/Mass tiers + a relay — ~24× cheaper than players — but they can't free-fly (no chunks). On pay-to-serve, spectator slots are a predictable capacity add (priced ~like player slots on bandwidth), behind an **allow-spectators toggle** (off / friends-only / open) + slot cap, set at server setup.

Driver B (live) and the Director (replay) are **complementary**: B is real-time with your body still playing and multiple simultaneous angles (streaming/casting); the Director is after-the-fact, choreographed, re-timed (polished cinematics).

### Driver C — the Director (offline replay playback + direction)

Loads a replay file and flies the Cinematic Camera (all modes) through it. **No re-simulation** — playback re-applies recorded states to a render-only world, so no determinism problem. No live-scouting risk (it's after the match).

**Two independent clocks — the key concept.** Everything below follows from separating:
- **Scene time** — where you are in the recorded footage (the timeline you scrub).
- **Shot time** — the camera's own movement along its path (keyframe A → B).

Keeping them independent is what enables bullet-time, slow-mo, and speed ramps.

**Authoring a dolly (the keyframe workflow):**
1. Scrub to the moment (the timeline scrubber); pause or leave it playing.
2. Fly the camera (free-fly) to the shot's start.
3. **Drop a keyframe** — captures the camera's current position + orientation (+ FOV).
4. Move on, drop more keyframes. The Director **interpolates a smooth path** (Catmull-Rom/Bézier position, smoothed orientation).
5. **Preview** the path (play the camera move independent of the scene), then tweak.

**Per-keyframe controls:** position/orientation (re-fly + re-grab), **dwell/timing** (how long to reach it → per-segment speed), **easing** (ease-in/out), **FOV** (zoom during the move → dolly-zoom/vertigo), **roll** (dutch tilt). Orientation can be keyframed **or** delegated to a **look-at target** (camera positions along the dolly but always aims at a chosen player/entity → effortless orbit/track).

**Speed control — two kinds (both present):**
- **Scene playback speed** — slow-mo (e.g. 0.25×), real-time, fast-forward, **pause**, reverse. Global timeline.
- **Camera path speed** — total shot duration + per-segment timing + easing.

Combine them: **bullet-time** (scene paused, camera orbits), **slow-mo action** (scene 0.25× + slow tracking dolly), **speed ramps** (scene speed changes mid-shot).

**Smooth despite 20 TPS.** The recording is captured at the 20 TPS sim rate, but the camera path is interpolated at full render framerate, and **recorded entity positions are interpolated between ticks** during slow playback — so both the camera move *and* the action stay smooth below 1×. (This is why "record at 20 Hz" still yields clean slow-motion.)

**Optionally bind keyframes to scene-time** (the Replay-Mod "time keyframe" idea) so a camera move is synced to the action — e.g. the camera arrives on the hero exactly as the explosion hits.

**Getting footage out:**
- **MVP:** the Director plays the shot **clean** (hide-HUD, smooth path) and the creator captures it with OBS / screen recording (normal, and unstoppable anyway). Zero new tech.
- **Later:** built-in **render-to-video** (encode frames to a file at a chosen resolution/framerate, possibly slower-than-real-time for max quality). Heavier (a video encoder); a future enhancement, not MVP.

### Recorders (source-agnostic, feed the Director)

The replay **file format is source-agnostic**: an initial snapshot + a timeline of deltas (entity transforms incl. player eye+yaw+pitch, block changes) with periodic keyframes, lz4-compressed. Whatever produces it, the **same Director** plays it back. Three sources:

- **Single-player** — capture the local sim's state deltas (SP already runs the full sim locally).
- **Multiplayer, client-side** — tee the inbound `StateUpdate` stream the client already receives. Zero server cost, but only captures **what was streamed to you** (your view distance).
- **Multiplayer, server-side (the headline)** — the server logs the authoritative full session and makes it downloadable. This is what delivers "follow anyone, anywhere." Heavier (storage + a download path + operator opt-in).

**Record-live / direct-after** (Axolittle's framing): recording is a passive capture during live play; *all* the heavy direction happens offline against the file. This is what makes MP replay tractable — the client/server is already producing this data; recording just keeps a copy.

## Use cases (what people actually do with it)

| # | Scenario | Driver | Modes used | When |
|---|---|---|---|---|
| 1 | **Build tour / world showcase** (solo) | A — own camera (SP) | Free-fly + path/dolly, hide-HUD | Live |
| 2 | **Gameplay B-roll of myself** | C — Director (my SP recording) | Follow-cam / POV on me, dollies | Replay |
| 3 | **Hero / bullet-time shot** | C — Director | Scene paused + orbit (look-at), slow-mo ramp | Replay |
| 4 | **Friend films me while I play** | B — spectator-camera | Free-fly / follow-cam on me | Live |
| 5 | **Multi-cam live shoot** | B — several spectator-cameras | Mixed; cut between angles | Live |
| 6 | **Tournament casting** | B — spectator-camera (delayed) | Follow players, cut POVs, wide tripod | Live (delayed) |
| 7 | **Watch the match from any angle** | C — Director (MP server recording) | Follow anyone, POV anyone, dollies | Replay |
| 8 | **"See what the winner saw"** | B or C | First-person POV | Either |
| 9 | **Promo / trailer flythrough** (outreach) | A or C | Path/dolly, FOV, hide-HUD | Either |
| 10 | **Fixed wide shot of a battle** | Any | Tripod | Either |
| 11 | **Screenshots / promo stills** | A — own camera | Free-fly + hide-HUD + zoom | Live |
| 12 | **Collaborative direction** (one plays, one directs) | B (live) or C (after) | Any | Either |

Deliberately small in primitives — **6 modes × 3 drivers × 2 timings** — but it covers the whole spread. Most "new" use cases are a new *combination*, not new code.

## Controls / interaction model

PC keyboard + mouse is the primary surface (per platform priority); touch is secondary and gamepad is parked. Two control surfaces — and the *camera fly + mode + target* controls are **identical** across both (and across all three drivers), so you learn the camera once.

### Surface 1 — In-game camera (Driver A live, or Driver B spectator)

A HUD-light overlay you toggle into; gameplay input is suppressed while active (like the existing inventory/explorer modals).

- **Toggle camera mode** — a single configurable key (default TBD; F5 = 3rd-person cycle, F1 = hide-HUD are taken — likely **F6** or a dedicated "Camera" bind).
- **Fly:** WASD = move, mouse = look, Space/Ctrl = up/down, Shift = boost; scroll = fly-speed (or FOV with a modifier).
- **Keyframes / tripods:** drop keyframe (key), clear, play path; **save/recall tripod on number keys** (Freecam-style: modifier + 1–9).
- **Target modes:** cycle target (next/prev player) for **follow-cam** / **look-at** / **POV**; toggle each.
- **Auxiliary:** hide-HUD, zoom (hold), roll.
- **Exit** back to play (Driver A) or stay in camera (Driver B).
- A small on-screen panel shows mode, target, fly-speed, and (spectator) the delay indicator + "● filming".

### Surface 2 — The Director (Driver C, offline editor)

A full editing UI (egui), modelled on conventions creators already know:

- **Timeline scrubber** — drag, or **J / K / L** (rewind / pause / play, tap-to-ramp), arrow keys to step frames.
- **Scene speed** — presets (0.25× / 0.5× / 1× / 2× / 4×) + a slider; pause; reverse.
- **Camera** — the same fly controls as Surface 1.
- **Keyframe panel** — add / select / edit (position, dwell, easing, FOV, roll) / delete / reorder; **preview path**; toggle **bind-to-scene-time**.
- **Target picker** — click an entity (or cycle) to set the follow / look-at / POV target.
- **Hide-HUD** for final framing; **export** (MVP: clean playback for screen capture; later: render-to-file).

Bindings are configurable; defaults follow game (WASD / F-keys) and editor (J-K-L / scrub) conventions so both gamers and creators feel at home.

## Cost & billing model (pay-to-serve servers)

Recording's marginal **compute is near-zero** — the server already builds + serializes the `StateUpdate` stream for networking; recording tees bytes already produced (off the hot path, background flush, never stalls the tick). The real cost is **storage** (and on-demand download bandwidth). (Live spectator-cameras are the separate, slightly-more-real cost — see Driver B; they're metered like player slots.) So for *recording*:

- **Operator toggle**, set at server spin-up (Setup Wizard step), with tiers:

  | Toggle | Behaviour | Storage | Burn-rate uplift |
  |---|---|---|---|
  | **Off** (default) | No recording | — | 0% |
  | **Clips** | Rolling buffer; persist only on "save that" | Bounded (saved clips only) | small |
  | **Full session** | Continuous; retained N days, capped + auto-prune | Capped | larger |

- **Burn-rate uplift = a predetermined %** (the billing UX — predictable, no surprise bills). **Size the % from a real storage measurement**, not a guess: record a busy session, measure compressed MB/hr, price storage + amortized bandwidth, divide by the server's hourly compute cost, add margin. Calibrate before locking numbers.
- **Default posture: rolling-buffer "clip it" + retention cap** — keeps real cost small and bounded so the flat % stays honest, and recording can't quietly balloon spend.
- **Downloads offloaded to blob storage** (Blossom/Beacon) — the game server records to a file that then lives in blob storage, so server bandwidth stays flat no matter how popular a clip gets. Files **encrypted**, view keys granted per access level (non-custodial, on-brand).
- **Operator opt-in, operator pays** — consistent with "operators run monetised servers"; the platform still never touches funds (just a higher metered rate for that server config).

Client-side recording = zero server cost but partial coverage; server-side = small cost, full coverage; rolling-buffer-save-on-demand = the sweet spot.

## Access model — two axes

A replay (and a live spectator feed) is **footage of other people**, so "who can access it" and "consent of who's in it" are separate concerns.

### Axis 1 — Access (operator's choice, set at spin-up)

Decided during server setup (Setup Wizard), **stated to every player at join** (Server Card disclosure) — so there's never a mid-session "gotcha." Changing it later requires a deliberate reconfigure that re-discloses; never a silent live flip. The same ladder governs **who may spectate (Driver B)** and **who may access a recording (Driver C)**.

| Level | Who can access / spectate | Default? |
|---|---|---|
| **Owner-only** | The operator | ✅ default |
| **Whitelist** | Named npubs | |
| **Participants** | Anyone whose npub was in the match (each gets their own cut) | |
| **Public** | Anyone | opt-in + guardrails (below) |

Enforcement is npub-based (the recording contains every participant's npub). Files encrypted in blob storage; view keys per level. **Operator access ≠ unrestricted redistribution** — publishing publicly still respects the Axis-2 consent layer (else owner-only recording becomes a backdoor to public footage of kids).

### Axis 2 — Identity / consent of people *in* it

Principle: **we can't police a camera pointed at a screen (screen capture is unstoppable in any game) — but the platform's own server recording / spectator feed is the powerful tool (all-seeing, downloadable, re-cuttable, follow-anyone), and *that* we can make respect people's privacy.**

Two **independent** switches per player — **nametag** (display handle) and **npub** (cryptographic identity, which links to their Lightning wallet + cross-platform identity graph). They carry very different risk and are gated separately:

| Age (verified) | Nametag in recording | npub in recording |
|---|---|---|
| **Under 13** (COPPA) | Never | Never |
| **13–15** | **Parent-enabled** (default off; the parent opts the child in) | **Never** (until 16) |
| **16+** | Own choice (default off, opt-in) | Own choice (default off, opt-in) |

Rationale: kids love seeing their handle in their favourite creator's video, and a chosen nametag is already visible live — so it's on the table from 13, but the **parent** (consent-holder with authority *and* judgment about permanence) makes that call, not the child alone. The **npub** is the genuinely dangerous one (wallet + identity linkage, no upside a kid wants) → locked until 16.

Hard requirements:
- **Verified age, not self-declared.** The toggles are gated on a **verified ≥16 / ≥13 credential** (Signet / HEAA age path). **Fail-safe default: anyone not verified is treated as the youngest tier → full pseudonymization.** This is what turns "kids are always Player 2" from honor-system into something that holds.
- **"Show my name" travels with the join** as a per-user preference (default off), so the recorder knows whether to alias each player.
- **True pseudonymization** for anyone hidden: the shareable file stores a throwaway "Player N" alias; the real npub is **not** in the artifact. Access control still works via a **separate, private npub→alias map** held encrypted by the server/owner — real identity for *access*, alias for *display*.
- **Children**: no chat (already the case), no nametag, no npub — a child in a replay is "Player N," full stop.
- **Display name is decoupled from npub in the file**, so showing a 13–15 nametag (parent-enabled) never drags their key along.
- **Takedown / right-to-be-forgotten** path: a participant or parent can request exclusion (identity-scrub + removal).

## Web spectate via weblink (Driver B, browser)

A browser is a first-class **spectator** reached by a shareable URL — the friend who doesn't want to install anything just clicks a link and watches. Full design in the implementation plan; the shape:

- **The operator's dedicated server is the sole token authority.** It mints a `SpectateToken` (volatile, in-memory, cleared on restart) via a signed admin command or the operator console, and assembles a self-describing URL: `https://axenstax.app/watch#e=<wss-endpoint>&t=<token>`. The secret lives in the URL **fragment** (`#…`), which browsers never send in the HTTP request — so the platform game-site never sees, stores, or validates it. Keeps "platform holds nothing / self-host = operator liability" intact.
- **`/watch` is a dumb shell** on the game-site: it serves the same WASM bundle with a `AXENSTAX_SPECTATE_MODE` flag and an inline boot script that reads the fragment, sets the dedicated-server endpoint + token, and boots the client in spectator mode (`PlayMode::Spectator`) — connecting `wss` straight to the operator's server.
- **The access ladder gates the join**: Public = token alone (anonymous); Owner/Whitelist/Participants additionally require a Signet `auth_event` matching the tier — reusing the Heartwood access policy already in `resolve_join_identity`. Expiry + a **separate** `max_spectators` cap (never consumes play slots) re-checked each tick.
- **Anti-X-ray is non-negotiable**: the spectator receives the **same obfuscated chunk stream** as players (buried ore → stone, applied in the shared server send-path before any bytes leave). A flying noclip spectator still can't scout, because the client never holds unobfuscated ore.
- **Privacy**: spectators are anonymous to the world (no avatar, excluded from mob targeting/collision/economy/save); participants see only a count ("👁 3 watching"); the `StateUpdate` is filtered (no reserve-richness / inventory / economy fields); chat is default-off for spectators — keeping passive viewing outside the economy/age perimeter.
- **WASM = the Close tier** (highest fidelity, reuses the full remote-render path). Standard/Mass tiers (`SpectatorSnapshot` + relay, Spec 04 §8) are deferred.

This rides on two foundations that also unblock real remote-multiplayer rendering: a **remote chunk send-path** and the **anti-X-ray obfuscation filter** — both built (with the spectator) in the plan's Phase 3.

## Build phases

1. **Cinematic Camera** — all six modes (free-fly, path/dolly, tripod, follow-cam, look-at, POV) + hide-HUD + zoom/roll, driven by **your own camera (Driver A)**, live, all modes; Surface-1 controls. Foundation, and it lets *us* start shooting outreach footage immediately. Lowest risk (reuses the existing camera decoupling; follow/look-at/POV target the live players).
2. **Director + Recorders (Driver C)** — replay file format (source-agnostic), the Director (Surface 2: timeline, two-clock speed, keyframe authoring, look-at, POV), then recorders in order: **SP** (prove the format) → **MP client-side** → **MP server-side full-session**. Designed MP-capable from day one so MP isn't a retrofit.
3. **Live spectator-cameras (Driver B)** — the networked **Close-Spectator** tier (Spec 04 §8, currently deferred): a friend joins and drives the camera live. Carries the obfuscated-stream + operator-gating + optional-delay work, plus the allow-spectators toggle + caps. (Bigger than Phase 1 because it needs the networked spectator path; the camera itself is reused from Phase 1.)
4. **Operator / billing / safety layer** — the recording cost toggle (rolling-buffer default, retention cap, blob-offloaded encrypted downloads), Setup-Wizard integration, Server Card disclosure, the access ladder, and the verified-age identity model + parental controls.

**Gating rules:** the Axis-2 identity/consent layer (verified-age gating, true pseudonymization, parental controls) **must ship before** the Public access tier, any MP server-side recording, or any spectator level beyond owner/whitelist. The spectator obfuscation + operator-gating must land *with* Phase 3, not after. Safety gating is not optional and is not a "later polish."

## Reuses (existing infra)

`Camera::render_eye` / `apply_free_look` / third-person modes · `physics::tick_flying` (spectator) · zoom FOV (#44) · `StateUpdate` deltas / `diff_entities` / `ProtocolId` · `PlayerState` (pos+yaw+pitch) · **Spec 04 §8 three-tier spectator protocol + relay** (`SpectatorSnapshot`, Close/Standard/Mass) · Server Setup Wizard · Server Card (declared privacy) · Operator Console · parent-controls-child accounts · Signet / HEAA age verification · Blossom / Beacon blob storage · NIP-17 / encrypted blobs · signed-claims.

## Risks / open items

- **Verified-age gating is the linchpin** of the kid protections — it has to be real (credential-backed, fail-safe to pseudonymized), not self-declared.
- **Dependency: the age path must expose *bands* (≥13 and ≥16), not just one gate.** The model needs to tell under-13 / 13–15 / 16+ apart. If the Signet/HEAA credential only proves a single threshold initially, fall back to the youngest tier for anything it can't prove (so an unprovable 14-year-old is pseudonymized, not exposed) and tighten as the credential gains granularity.
- **Public distribution of identifiable-minor content** is the sharp regulatory edge (UK OSA / Children's Code / GDPR / COPPA). The defaults are designed to stay on the right side of it, but the Public tier warrants the earmarked **counsel pass**.
- **Live spectator-cameras reintroduce a scouting vector** (they pull fresh chunks around the camera) — mitigated by the obfuscated stream + friends-only gating + optional delay, but it means spectators are a *trusted/gated* feature, not open-by-default. Building it also depends on standing up the networked Close-Spectator tier (Spec 04 §8), which is more work than the local camera.
- **Storage sizing** — measure compressed MB/hr on a busy session before locking the billing %.
- **File size** — delta + periodic keyframes + lz4 + length caps; rolling-buffer keeps persisted size to "what people saved."
- **Body-frozen-while-filming** (Driver A) in live multiplayer — accept + indicate ("● filming"); Driver B (friend camera) and Replay avoid it.
- **Render-to-video is a later enhancement**, not MVP — MVP output is clean playback the creator screen-records.
- **Codename** — pick one.

## Verification approach (per phase, when built)

- **Phase 1:** live native run — all six modes feel right (free-fly, dolly preview, tripod, follow, look-at orbit, POV); HUD-hide + zoom/roll; block-interaction gated; camera can't load distant chunks (anti-X-ray holds).
- **Phase 2:** record → play back in the Director; two-clock speed (slow-mo stays smooth, bullet-time orbit); keyframe authoring + look-at target; follow/POV track the right entity; round-trip an SP recording, then an MP client-side one.
- **Phase 3:** a friend joins as a Close Spectator and drives a live camera while you keep playing; the spectator sees the obfuscated stream (no buried ore); allow-spectators gating + optional delay behave; load stays bounded with N cameras + a speed cap.
- **Phase 4:** access-level enforcement (npub-gated, governs spectate + replay access); verified-age pseudonymization (unverified → Player N); parental toggle for 13–15 nametag; blob-offloaded encrypted download; Server Card discloses the policy at join.
- `./check.sh` green at each phase.
