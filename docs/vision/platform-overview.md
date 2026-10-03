# Axe'n'Stax — Supporting Platform Overview (Condensed)

> **The thesis: AxeNStax is sovereign gaming. Freedom first.**
>
> The product is *freedom* — you own your world, your identity, your creations, and your
> server. Everything else is downstream of that. *Because* you're sovereign, you **can** run
> your own economy, transact peer-to-peer, and earn real value from play — but the economy
> is a **side benefit that freedom unlocks, never the headline and never the reason to
> play.** Lead with sovereignty; Bitcoin comes after rapport.
>
> This ordering is deliberate and load-bearing:
> - It places AxeNStax in the **sovereign-platform** category — *out* of the play-to-earn
>   graveyard (Axie, the web3 cohort) where earning *was* the product and the game collapsed
>   the moment the earning stopped.
> - It matches the demand evidence: the durable, reachable, *retaining* demand is for
>   sovereignty and ownership (the #SaveMinecraft grievance; creators wanting to own their
>   economy). The earn-from-play hook is unproven, and the broad "next voxel sandbox" hype is
>   largely vapour (Hytale's headline traction numbers were fabricated/unverifiable).
> - It de-risks compliance and marketing: we sell *freedom to transact*, never
>   "earn free Bitcoin" (the FCA/ASA criminal-promotion line).
>
> **The one discipline this demands:** the economy can never become the retention floor. The
> solo, no-Bitcoin loop must be fully fun on its own — the game has to stand with Bitcoin
> switched *off*. The moment "people stay because they're earning" creeps in, we've rebuilt
> earn-to-play and we're back in the graveyard.
>
> Sources: `docs/research/2026-06-22-minecraft-competitor-failures-and-axenstax-strategy.md`
> (The Voxel Graveyard) + the 2026-06-22 demand study; memory `project_sovereign_gaming_positioning`.

## 0. Project Posture

**This is a moonshot. Go big or go home.**

After extensive research into existing voxel engines (Luanti, Veloren, Terasology, Cuberite, ClassiCube), the conclusion is clear: no existing engine can deliver this vision without becoming the ceiling. Axe'n'Stax is a **full custom build** — custom engine, custom client, custom server, custom protocol.

The research informed this decision (see `docs/research/`). The architecture patterns, cost models, and operational playbooks from that research remain valuable. But the engine is ours.

See: `docs/architecture/ADR-001-full-custom-engine.md`

---

## 1. Core Vision

The organising principle is **player and operator sovereignty** — own your world, your
identity, your creations, and your server. The platform exists to make that freedom real at
every scale, from a single offline world to a global event.

Build an open-source, massively scalable voxel sandbox platform capable of:

- Personal sandbox worlds (sleeping, near-zero idle cost)
- Social mid-scale worlds
- Large celebrity/event worlds
- True open-source self-hosting
- Player-owned identity (Nostr) and operator-owned economies — sovereignty by design
- Freemium economics with extremely low marginal cost for free users

This is not "a server". It is a World Runtime Platform — and, above that, a **sovereignty
platform**: the economic layer (Bitcoin, operator splits, real value) rides on top of the
freedom, as a consequence of it, never as the reason for it.

---

## 2. Strategic Objectives

### 2.1 Product Goals

- Anyone can spin up a personal world.
- Worlds can be opened to others instantly.
- Celebrity events can scale massively without cloning deception.
- Platform supports both tiny worlds and high-concurrency worlds.
- Open source by design — anyone can self-host.

### 2.2 Economic Goals

- Free tier must trend toward near-zero marginal cost.
- Most worlds idle most of the time.
- Cost scales primarily with active player-hours.
- Spectator presence should be significantly cheaper than interactive presence.
- Self-hosting reduces central infra burden.

---

## 3. World Types (Unified Architecture, Different Policies)

All world types run on the same engine.
Only scaling policies differ.

### 3.1 Personal Worlds
- 0–10 players
- Sleep when empty
- Cold start acceptable
- Storage-only cost when idle

### 3.2 Social Worlds
- 20–200 players
- Multiple regions
- Moderate autoscaling
- Stable tick guarantees

### 3.3 Event Worlds
- 500–10k+ presence
- Distributed region simulation
- Tiered presence model
- Pre-warmed capacity
- No deceptive cloning

---

## 4. Scalability Model (Non-Mega-World Philosophy)

The platform must support:

- Many independent sandbox worlds
- Larger high-concurrency worlds
- Occasional burst events

This is not a single global mega-world.

### 4.1 Region-Based Simulation (Long-Term Direction)

- World divided into regions (simulation units)
- Regions are authoritative
- Regions can migrate between machines
- Small worlds may run entirely in one worker
- Large worlds distribute regions across workers

This supports horizontal scaling without requiring a single monolithic server.

### 4.2 Portal / Zone-Based Scaling

Instead of deceptive world cloning:

- Explicit zones or dimensions
- Portal-based transitions
- Worlds can expand spatially via connected regions
- Maintains shared-universe feeling

This avoids seamless cross-server complexity initially while preserving scale.

---

## 5. Cost Model Insights (High-Level Summary)

### 5.1 Reality of Game Server Economics

- Cost scales roughly with concurrent users (CCU)
- Compute + bandwidth drive cost
- Typical optimized cloud cost: ~$1–$4 per CCU per month
- Idle worlds must not consume active compute

### 5.2 Freemium Feasibility Strategy

To approach near-zero marginal free-user cost:

- Worlds sleep when empty
- Aggressive autoscaling
- Tiered presence (interactive vs spectator)
- Optional self-hosting
- Use spot instances / cost-optimized orchestration

### 5.3 Spectator Mode (Critical Lever)

- Spectators significantly cheaper than active players
- Snapshot/stream-based presence
- Essential for celebrity-scale events

---

## 6. Open Source Commitment

Core principles:

- Entire engine open source
- Anyone can self-host worlds
- Creators can run their own infrastructure
- Platform layer (discovery, identity, orchestration) optional but value-added

This reduces central hosting burden and increases adoption.

---

## 7. Visual Strategy (Initial Commitment)

### 7.1 Texture Resolution Decision

- Default texture pack: 16×16
- Renderer must be resolution-agnostic
- Resource packs hot-swappable

Rationale:
- Fast AI generation
- Small downloads
- Familiar voxel aesthetic
- Minimal iteration friction

Higher-resolution packs (32×32, 64×64+) supported later without engine change.

### 7.2 2D UI Style (Menus, HUD, Inventory)

**The 2D interface is modern and clean, independent of the 16×16 block textures.**

The blocky aesthetic belongs to the 3D voxel world — it's a gameplay and rendering choice. The 2D layer (menus, HUD, inventory, chat, settings) follows a different design language: anti-aliased fonts, smooth panels, resolution-independent layouts, and proper DPI scaling.

#### Industry precedent

Every successful 3D voxel game uses clean modern UI on top of blocky worlds:
- **Hytale**: fully modern UI, explicitly designed as a "consistent visual language"
- **Deep Rock Galactic**: flat, themed UI with no pixel art
- **Veloren**: clean RPG panels (egui-based)
- **Vintage Story**: utilitarian but clean, readable fonts

Only 2D pixel-art games (Terraria, Stardew Valley) use pixel-art UI — because their *entire* visual style is pixel art.

#### Design principles

1. **Thematic, not pixelated.** UI echoes the game world through earthy colours, beveled edges, subtle block-texture accents — not through pixel-art rendering.
2. **Resolution-independent.** All UI uses logical units, not pixel coordinates. Scales cleanly from 720p to 4K.
3. **Moddable from day one.** The #1 lesson from Minecraft's Ore UI backlash: UI must be skinnable/customisable. Resource packs can override UI colours, fonts, and layout.
4. **Fast and responsive.** No lag, no choppiness in menus. UI interactions must feel instant.
5. **Accessible.** Anti-aliased fonts with proper hinting. Readable at all DPI scales. Suitable for younger players on varied hardware.

#### Framework

The UI is rendered using `egui` (immediate-mode GUI, cross-platform, WASM-compatible). See Spec 03 Section 8.

### 7.3 Texture Architecture Requirements

- Block IDs map to texture names (not pixel coordinates)
- UV mapping in normalized space (0–1)
- No hardcoded resolution assumptions
- UI scaling independent of texture size
- Avoid fragile atlas coupling

Textures are client-side only and do not materially impact server cost.

---

## 8. AI-Driven Development Approach

- Development driven from specification documents
- AI stack generates code and assets
- Strict architectural boundaries required
- Visual style guide must exist for consistent asset generation

Initial focus is engine correctness and gameplay feel, not visual fidelity.

---

## 9. Initial Build Philosophy

Ship fast, iterate fast.

Early alpha priorities:

- Movement feel
- Block placement responsiveness
- Chunk streaming smoothness
- Multiplayer stability
- World persistence

Visual polish can iterate independently.

---

## 10. Local Game First, Online Platform Second

Axe'n'Stax ships as a **standalone game** that works offline. Download it, run it, play solo — no server, no internet, no account required. Just like vanilla Minecraft.

- **Solo/LAN mode**: Single binary. Client + embedded server in one process. World saved locally. Invite friends over LAN.
- **Online mode**: Connect to dedicated servers (cloud-hosted or self-hosted). Bitcoin features, persistent worlds, events, platform services.

Same engine, same world format, same gameplay. A world started solo can be uploaded to a hosted server. Nothing changes except where the server runs.

The game must be great *before* anyone ever goes online — and great with **Bitcoin switched
off**. This is the freedom-first thesis in practice: sovereignty (own it, run it, play it
offline) is the floor; the economy is opt-in upside layered on top. If the solo, no-Bitcoin
loop isn't fully fun on its own, nothing built above it is sound.

---

## 11. What This Is (And Is Not)

This project is:
- A standalone voxel game that works offline
- A sovereignty platform — you own your world, identity, creations, and server
- A scalable online sandbox platform
- Open-source by design
- Freemium-capable
- Architected for both small and massive worlds

This project is not:
- An online-only game
- A single mega-world MMO
- A closed hosting service
- A texture-driven product
- **A play-to-earn / earn-to-play game** — earning is a freedom-unlocked side benefit, never the point

---

## 12. Summary Position

Axe'n'Stax aims to:

- Outscale technically when required
- Enable community-hosted growth
- Use smart orchestration to control cost
- Separate simulation cost from audience size via tiered presence
- Deliver a voxel sandbox that can scale from 1 player to global events

The foundation must prioritize:

Scalability architecture > gameplay systems > visual fidelity.

Resolution: 16×16 default, resolution-agnostic renderer.
World model: sandbox-first, event-capable.
Economic model: freemium via cost compression and self-hosting support.

