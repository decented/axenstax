# Foundation — Glimkin Mod for AxeNStax

**Status:** READY TO BRAINSTORM (pre-build; engine prerequisites unwritten)
**Date:** 2026-05-19
**Author:** Staxolottle (design) + Claude (technical translation)
**Vision parent:** [Player-Driven Economies — Long-Run](../vision/economies-long-run.md) §9.5 (Plugin / Mod Marketplace, T5+)
**Sibling project:** [Glimkin](../../../Decented/Glimkin) — physical-NFC creature game (Decented platform)
**Sibling specs:** Spec 1 §8 (Plugin Architecture), Spec 6 (Bitcoin / Charter), `2026-05-14-engine-signing-bridge.md`
**Cross-product reference:** `Decented/Glimkin/docs/game-design.md`, `creature-encyclopedia.md`, `biome-system.md`, `visions.md`, `federated-verifier-brainstorm.md`

---

## TL;DR

The Glimkin mod runs in two modes that coexist:

- **Mode A — Realm of Strangers.** AxeNStax becomes a second window onto the same other realm Glimkin's physical NFC cards open. Discovery, glimpses, witness events signed and counted by the federated verifier. The geocaching-extension model.
- **Mode B — Realm of Bonds.** AxeNStax becomes a personal realm where the creatures you've *already* witnessed — through any source — manifest as ambient inhabitants. Your Blazehart drifts near campfires; your Crysthorn sits in a forest. You don't witness them again (the veil parts only once); you live alongside them. Their rendering clarity tracks their resonance — bonds you've deepened in the Glimkin app become visibly clearer in voxel.

Mode A populates Mode B. Every new glimpse becomes a new inhabitant. Legacy fades them out. Glimkin's existing turn-based combat (`game-design.md` §6) can be **staged** in a voxel arena as shared vision — voxel as theatre, Glimkin as the script.

The mod preserves Glimkin's three load-bearing constraints — **witness, don't hoard**; **effort over computation**; **gentle, anti-FOMO tone** — by translating them into voxel mechanics that are *structurally analogous*, not skinned versions. Real-world travel becomes in-world traversal of range-class distance. Hardware-enforced NFC write-protection becomes Nostr-signed portal blocks that cannot be reset until Legacy. NFC-card forging becomes in-world Portal Seed crafting with the same 48-hour attunement and no-self-glimpse rule.

The mod also unlocks a strategic outcome neither product can deliver alone: a **digital-native onramp to Glimkin for players without physical cards**, while preserving Glimkin's physical-first ethos because the real-world layer still gives access to biome diversity and witness networks that no single voxel server can fake.

---

## 1. Why this exists

### 1.1 The shared-realm thesis

Glimkin's central conceit is that creatures live in a parallel realm, always there, glimpsed through portals. The physical NFC card is *one kind* of portal. There is no design reason it must be the *only* kind. The Glimkin game-design doc already accepts forge-able portals on blank cards; portals also exist as Static Locations (Sanctuaries, Arenas) on the in-app map. The voxel mod adds a third portal kind — placed inside an AxeNStax world — that obeys the same realm rules.

The mod does not invent new creatures. It does not extend the journal schema. It does not run combat. It is a **second observation channel** for the same creatures Glimkin already manages. This is the design discipline that protects Glimkin's brand.

### 1.2 Why this is the right first mod for AxeNStax

The AxeNStax mod story (Spec 1 §8) needs an early showcase that justifies four claims at once:

| Claim | How this mod proves it |
|---|---|
| Mods can use Nostr identity end-to-end without rebuilding it | The mod's only auth is the player's existing Signet npub; witness events are signed by the same key the player uses for Charter and for Glimkin. |
| The capability model is real | The mod requests `block_registry`, `entity_spawn`, `world_read`, `network_http`, `nostr_sign`, `visual_overlay` — and explicitly does NOT request `world_write`, `player_inventory`, or `filesystem`. The audit story is clean. |
| Decented is a platform, not a single game | A Glimkin journal entry earned inside AxeNStax is the most concrete cross-game artefact we can demonstrate. |
| Mods can change a game's voice | Glimkin's brand requires *quiet*. The mod's UX inside AxeNStax must be markedly gentler than the host game's defaults. Server operators see that a mod can alter tonal experience, not just mechanics. |

### 1.3 The two-mode framing

The mod has two modes that coexist, not alternatives. Both are first-class. The doc is structured around Mode A for sections §3–§12 (it's the discovery surface and the harder-to-design half), then Mode B in §23 (the personal-realm experience) and §24 (voxel-as-arena for battle).

**Mode A — The Realm of Strangers.** The public surface. Portals manifest in worlds, you tap them, the veil parts, you witness creatures you've never bonded with before. This is the geocaching-extension layer, the federated witness amplification, the cross-server discovery. New journal entries originate here.

**Mode B — The Realm of Bonds.** The personal surface. Your AxeNStax world becomes a place where the creatures you have *already* resonated with — through any source — manifest as ambient inhabitants. You don't witness them again (the veil parts only once). You live alongside them. They render at the same blur level Glimkin's app uses for the same resonance band, so deepening a bond in the app makes your voxel companion clearer.

**Mode A populates Mode B.** Every new witness becomes a new inhabitant. Legacy fades them away.

**The lore framing the brainstorm crystallised:** the voxel game is, in lore terms, *another realm* — a block-built reality with its own physics, alongside the real world and alongside the Glimkin other-realm. The veil between them is what the mod has been describing. Mode B is the gaze inverted: instead of looking *through* the veil at the other realm, you bring already-bonded creatures *into* your voxel realm. They cross the veil because they know you. The bond is the bridge.

This dual framing changes the centre of gravity of the mod. Discovery (Mode A) is the *recruiting* surface. The personal realm (Mode B) is what players come back for — the feeling of walking through a world that knows them.

### 1.4 Strategic outcome for Glimkin

Glimkin's physical-first ethos has a real cost: players who don't have access to physical cards (most of AxeNStax's alpha kid audience) currently cannot play. The voxel mod opens a **fully digital path** that is gameplay-equivalent — same journal, same creatures, same witness rules — without diluting the physical product, because:

- Real-world Glimkin still grants biome diversity (Tundra, Tropical, Desert in the UK = abroad-only) the voxel can't reproduce
- The social geocaching layer is irreplaceable
- Voyager / Pilgrim range classes inside a single voxel world hit a soft ceiling (you can only walk so far in one seed)
- Physical-realm witness density is naturally higher than single-server voxel-realm density

So players who start in AxeNStax have reasons to graduate to physical cards. Players who own physical cards have a daily-driver digital surface between trips. Neither product cannibalises the other.

---

## 2. Core principles (the four guardrails)

These are non-negotiable. They override anything else in this spec when in conflict.

1. **Witness, don't hoard.** No creature ever enters a player inventory. No creature is tradeable. No creature has a sat price. The mod explicitly refuses participation in Vendor Block, crafting, combat, or anti-cheat hash exposure.
2. **Effort over computation.** A glimpse requires the player to have *moved* to the portal location through the voxel world. The Range Class distance rules are enforced server-side and are calibrated to be meaningful at AxeNStax's player walking speed.
3. **The veil only parts once.** Per pubkey, per creature, across all source channels. If you have glimpsed Blazehart #4729 via a physical card in Bristol, the AxeNStax portal of the same creature shows you the "Previously Witnessed" screen and grants no new witness credit. This rule is the spine of Glimkin's anti-farming design and must hold across realms.
4. **Gentleness in voice.** No XP popups, no "+1 witness" toast, no level-up sound, no leaderboards, no scarcity countdowns. Glimkin's brand voice (`brand-identity.md` §Voice Guide) governs every player-facing string. Errors use Glimkin lore phrasing.

---

## 3. Mechanic Translation: physical → voxel

The full mapping. Anything not in this table is out of scope for MVP.

| Glimkin (physical) | Glimkin Mod (voxel) | Why this preserves the spirit |
|---|---|---|
| NTAG 424 DNA NFC card | **Portal Block** — placeable, element-tinted, breathes slowly | Same role: a thing in the world that, when interacted with, opens a portal. |
| AES-128 chip-level write protection | Block carries a signed-by-creator manifest; server refuses replacement until Legacy | Translates a hardware constraint into a Nostr-enforced one. Same anti-griefing outcome. |
| Hide in a real place (park, library, picnic table) | Place inside the voxel world; chunk biome auto-recorded | Same role: a specific place tied to an environment. Biome categorisation uses Glimkin's existing 15-biome taxonomy (§5 biome alignment). |
| Tap with phone | Right-click the block once | A glimpse is a single act of attention. |
| Portal collapses on observation | Block goes inert; cannot reopen for the witness | Veil-only-parts-once enforced at the per-block level. |
| Rehide ≥ 500m (Homebody), ≥ 1km (Wanderer), ≥ 5km (Voyager), ≥ 20km (Pilgrim) | Carry inert **Portal Seed** ≥ 200, 2000, 5000, or 20000 voxel blocks; place in biome-eligible spot | Voxel walking speed and view-distance calibrate these distances to feel like the physical equivalents. Pilgrim portals genuinely require cross-region travel. |
| Glimpse a creature | Transient billboard-sprite entity walks across view for 5–6 seconds, then fades. No drop, no pickup. | The creature is *witnessed*, then gone. Sprite uses the original Glimkin 2D art — on-brand and clarifies that you're not catching anything. |
| Vision (1-in-5 by hash) | Chunk-area atmospheric overlay (fog, lighting, skybox tint, ambient audio) for ~10 seconds | Glimkin's visions are static landscape art; the voxel translation is environmental retexturing rather than a pop-up image. |
| Witness event signed and published | kind-30900 Nostr event signed with player npub, broadcast to configured relays | Identical schema. The federated verifier doesn't care which channel sourced it. |
| 48-hour attunement after forge | 48 in-world hours (subject to server `/time speed`); during attunement the portal shows as a faint pulse with no element | Time-pressure for forge-and-hide preserved. AxeNStax's `/time` controls feel cohesive across the mod. |
| Cannot be first to glimpse your own portal | Same rule, enforced by server checking the genesis event author vs glimpser pubkey | Prevents self-witnessing and forces social play. |
| "The veil held firm this time" — failure rate | Even on a valid portal, the HMAC roll can return `veil_holds`; player sees the same lore message | Glimkin's mystery-not-error principle. ~10% baseline, modifiable by player's veil-thinness score. |
| Static Locations (Sanctuaries, Arenas) | Server-admin-placed **Anchor Blocks** that always show portals when conditions allow | Curated discovery sites; the in-game equivalent of the in-app static map markers. |
| Forge from blank NFC card | Craft **Portal Seed** from rare materials + Nostr-signed genesis ritual | Players can forge new portals in-game; cross-products with AxeNStax's existing crafting system. |
| Reduced-precision geohash in events | Biome tag only; no chunk coordinates, no world ID without consent | Glikkin's location-privacy invariant inherited end-to-end. |

---

## 4. The Veil — environmental thinness as a system

This is the deepest design move. It replaces "loot-based" portal discovery with **environmental physics**, and it's load-bearing for the brand.

### 4.1 The model

The "veil between realms" is given a numerical thinness score per chunk per tick. When thinness crosses a threshold near a player, portal manifestation events become possible. Players never see the score directly. They notice the world feels different — a slow drop in ambient sound, a faint particle drift, a hush in the wind track.

Thinness accumulates from:

| Source | Contribution | Rationale |
|---|---|---|
| Dawn / dusk transitions | +0.20 for ±15 in-game minutes around each | Liminal time. Folklore consensus. |
| Player session length | +0.05 per continuous in-game hour, max +0.30 | Long expeditions earn moments. |
| Distance from spawn | +0.10 per 1000 blocks, max +0.30 | Rewards travel without forcing it. |
| Biome diversity in last 30 min | +0.05 per unique biome traversed, max +0.20 | Mirrors Glikkin's Wit-from-diverse-biomes mechanic. |
| Number of other players in same chunk | +0.15 per additional player, max +0.45 | Social witnessing is amplified. |
| Standing at a biome boundary | +0.10 while within 8 blocks of the line | Edges are thinner. |
| Standing in a cave entrance | +0.15 while liminal | Same. |
| Recently glimpsed (last 5 min) | −0.50 cooldown | Prevents glimpse-chaining. |
| In creative mode | thinness disabled entirely | Creative is for builders; portals are a survival mechanic. |

Thinness threshold for spontaneous events: **0.60**. Threshold for portal manifestation at an Anchor Block: **0.40** (anchors lower the cost).

### 4.2 What thinness produces

Three event classes, each with their own threshold and probability roll:

| Event | Threshold | Per-tick roll | Effect |
|---|---|---|---|
| **Portal Manifestation** | 0.40 (anchor) / 0.60 (wild) | 1 in 600 ticks (~30s) | A Portal Block appears within sight; lingers up to 10 in-game minutes if untapped |
| **Vision** | 0.50 | 1 in 1200 ticks (~60s) | Chunk-area atmospheric overlay; one of the 30 Glimkin visions chosen by biome match |
| **Pulse** | 0.30 | 1 in 200 ticks (~10s) | A faint audio cue and particle hint — "something stirs on the other side"; no entity spawned. Pure ambience. |

Pulses are the most frequent and intentionally cheap — they keep the mod's presence felt without being a slot-machine. The brand needs gentleness, not engagement-bait.

### 4.3 Why this isn't a loot system

Crucially, the player *cannot grind thinness*. Standing still doesn't help. Killing mobs doesn't help. Mining doesn't help. Only travel, time-of-day, biome variety, and friends help — and all of these are themselves desirable behaviours we want to reward. Thinness is the ambient consequence of *playing the game well*, not a metric to optimise.

This is the philosophical separation from "ore distribution + RNG" — the mod refuses to be discoverable through grinding.

---

## 5. Biome Alignment

Glikkin's 15-biome taxonomy (`biome-system.md`) is the canonical biome list for cross-game witness events. AxeNStax must report glimpses against it.

### 5.1 The mapping

| Glikkin biome | AxeNStax detection (today / once worldgen lands) |
|---|---|
| Urban | Player-built area with ≥ 20 placed blocks within 16 |
| Parkland | Grass surface + ≥ 5 placed flowers/sapplings within 16 |
| Forest | Forest biome (or, today, ≥ 8 trees within 16-block radius) |
| Grassland | Plains biome (or, today, grass surface + no trees within 16) |
| Farmland | ≥ 4 tilled soil blocks within 16 |
| Coastal | Within 8 of a beach/ocean boundary |
| Riverbank | Within 4 of a river/lake/pond |
| Wetland | Swamp biome (or, today, mud/lily/reed presence) |
| Mountain | Y ≥ 96 and surface stone |
| Ruins | Server-admin-flagged Ruin marker block within 32 |
| Underground | Y < surface − 16 |
| Desert | Desert biome (or, today, ≥ 8 sand blocks within 16, surface) |
| Tropical | Jungle biome (post-T5 farming worldgen) |
| Tundra | Tundra biome (post-T5 farming worldgen) |
| Island | Surface land surrounded by ocean within 64 in all cardinals |

### 5.2 The biome worldgen dependency

AxeNStax's current worldgen emits a single biome in practice (per `farming-economy-long-run.md` §"biome variety" note). The mod cannot perfectly tag glimpses against Tropical / Tundra / Forest / Desert until the biome worldgen spec lands.

**MVP strategy:** use the heuristic column above (block-presence + Y level + adjacency), which works on the current single-biome map. When biome worldgen lands, swap the inference for direct biome reads — the host-side `biome_at(pos)` function abstracts the change.

This is a soft dependency, not a blocker. MVP launches with heuristic detection. Server admins can also place biome-declaration blocks to override.

### 5.3 Element-biome resonance

Glikkin's element-biome resonance table (`biome-system.md`) carries through verbatim:

- Fire portals are more likely to manifest in Desert / Urban / Ruins biomes
- Water portals — Coastal / Riverbank / Wetland
- Earth — Mountain / Forest / Farmland
- Air — Grassland / Mountain / Island
- Electricity — Urban / Desert / Tundra
- Ether — Ruins / Underground / Island

The mod's portal-manifestation logic biases roll outcomes by this table. Same numbers Glikkin uses (+15% to the resonant element's appearance weight).

---

## 6. Visions — 30 of them, faithfully

`Decented/Glimkin/docs/visions.md` defines 30 visions across the 6 elements, with art prompts and biome affinities. MVP ships 5 of them, one per element, mapped to AxeNStax-detectable biomes:

| # | Vision | Element | Biome | AxeNStax trigger condition |
|---|---|---|---|---|
| 1 | The Ember Fields | Fire | Desert | Standing in sand + day + Fire portal nearby OR raw thinness ≥ 0.70 |
| 7 | The Mirror Confluence | Water | Riverbank | Within 4 of water + dawn/dusk |
| 11 | The Rootwall | Earth | Forest | Within 16 of forest-dense area + Y near surface |
| 20 | The Stillpoint | Air | (Heartland) | High Y (≥ 96) + zero wind audio (no movement input for 60s) |
| 21 | The Spark Grid | Electricity | Urban | ≥ 20 player-placed blocks within 16 + storm time |

Full 30 ship in subsequent waves. Each vision is a chunk-area lighting + fog + skybox + ambient-audio swap lasting ~10s, then fading. No player input is captured during the vision — it's purely atmospheric.

**Vision journal entries** are first-class — same Nostr event schema as creature glimpses, with `["kind", "vision"]` instead of `["kind", "creature"]`. They count toward the player's journal but not toward any creature's clarity (per `visions.md` §Journal Entry).

---

## 7. Discovery vectors — five paths to a portal

A single discovery mechanism would be brittle. Glikkin's physical layer has multiple paths in (Genesis Drops, Keeper Kits, DIY blanks, finding-someone-else's-card). The voxel mod mirrors this with five.

### 7.1 In-world ambient manifestation

The default. Veil thinness crosses 0.60 in a chunk, the manifestation roll succeeds, a Portal Block appears within view, drawn from a pool of currently-attuned creatures across the configured verifier set. Visible for up to 10 in-game minutes if untapped. No HUD indication — the player has to be paying attention to the world.

### 7.2 Anchor-bound manifestation

Server operators place **Anchor Blocks** during world-build. Each anchor lowers the local thinness threshold to 0.40 and acts as a portal magnet — manifestation rolls there are 4× more frequent. Like geocaching cache locations: known sites where the realm is reliably thin.

Anchors are placed by server admins only (no recipe; available via `/give` to ops). They cost nothing to place, but each anchor reduces the spawn weight of other anchors within 256 blocks — encouraging spread.

### 7.3 Player-forged portals

The full Glikkin Forge → Hide → Wait loop, ported. Players craft a **Portal Seed** from rare materials + a Nostr-signed genesis event. Place it. 48 in-world hours of attunement. Cannot be first-glimpsed by the forger. First glimpse establishes the creature.

Forge recipe (T1.x — refines later):

```
[ FF ][ -- ][ FF ]
[ -- ][ NS ][ -- ]    FF = Filament Fragment (rare drop from any ore vein)
[ FF ][ -- ][ FF ]    NS = Nostr Signature (consumed; signed in-app)
```

The Nostr-signature ingredient is interesting: it requires the player to authenticate a one-time event before the recipe succeeds. This is the in-game Forge action. The signed event becomes the creature's genesis event on Nostr — same role as the physical-card init.

### 7.4 Physical-bridge portals

When a player taps a real Glimkin NFC card, the canonical PWA (`glimkin.world/g`) — beyond performing the physical glimpse — mints a one-time **Glimpse Token** (a kind-30901 short-lived Nostr event signed by the player). The token references the physical card's creature.

The next time the same player joins a Glimkin-mod-enabled AxeNStax world, the server sees the token, marks it spent, and queues a portal manifestation for the same creature in a biome-matching chunk. This is the physical → digital bridge.

Token TTL: 24 hours. One token per physical glimpse. The token is not transferable.

### 7.5 Co-visit manifestation

When 2+ different player keys have stood in the same chunk within the same 7-day window in multiplayer, manifestation chance in that chunk doubles for the next 24 hours. The realm thins where players cluster. This is the in-voxel equivalent of physical Glimkin's hotspot dynamics — without leaking location to either player.

---

## 8. The Glimpse — what actually happens

The full sequence when a player right-clicks an active Portal Block.

### 8.1 Server-side gate (in order)

1. **Authentication.** Player has valid Signet auth (handled by AxeNStax already).
2. **Veil-once check.** Server queries the federated verifier: has this pubkey ever witnessed this creature_id, on any source? If yes, return `previously_witnessed` — render dimmed portal art + creature's current journey, but no event published, no journal change.
3. **Self-forge check.** Was this portal forged by the same pubkey? If yes, return `cannot_self_glimpse` — show the "Your Portal" UI.
4. **Attunement check.** Forged within last 48 in-world hours? If yes, return `still_attuning`.
5. **Veil-holds roll.** Compute `tap_hash = HMAC-SHA256(server_secret, creature_id || pubkey || tick || chunk_coords)`. With probability `0.10 + (1.0 - thinness) * 0.10`, return `veil_holds`. Brand voice: *"The veil held firm this time. It happens — not every moment is right. Try again when you're ready."*
6. **Vision vs creature branch.** Compute `vision_index = tap_hash mod 5`. If `vision_index == 0` and the chunk's biome is in the element's vision-affinity set, return a Vision result. Else return a Creature result.

### 8.2 Client-side cinematic (creature path)

The 5–6 second sequence is *the experience*. It must feel deliberate.

- **t=0.0** — Portal Block flashes once at its element colour. Ambient sound ducks to 30% volume.
- **t=0.5** — Faint particle drift radiating outward, element-tinted.
- **t=1.0** — Billboard sprite of the creature fades in 10 blocks beyond the portal, facing the player. Translucent at first.
- **t=2.0** — Creature "walks" — sprite drifts laterally across the player's view, 3 blocks across, opacity peaks at 70%.
- **t=4.0** — Sprite begins to fade. Element colour cools.
- **t=5.5** — Sprite gone. Ambient sound returns. Portal Block goes inert (visibly dimmer; cannot be tapped again by this player).
- **t=6.0** — A single soft chime. Journal entry appears in the player's book if it is open. Otherwise: nothing. No toast. No popup. The player must check.

The sprite is the original Glikkin 2D art at the appropriate resonance band (rendered with biome-modifier accumulation already applied — see `image-catalogue-spec.md`). Voxel-translating the creature would break the brand: a Glikkin is something you *glimpse*, and a chunky voxel mob is not what a glimpse looks like.

### 8.3 Client-side cinematic (vision path)

- **t=0.0** — Sky tint begins to shift toward the vision's palette.
- **t=1.5** — Fog density ramps up. Ambient lighting recolours.
- **t=3.0** — Vision peaks. Skybox replaced by the vision's art prompt rendering (or a pre-baked HDRI for MVP); ambient audio crossfaded to vision-specific track.
- **t=8.0** — Reverse fade begins.
- **t=10.0** — Normal world restored. Journal entry recorded silently.

### 8.4 Journal entry

```jsonc
{
  "kind": 30900,
  "tags": [
    ["d", "<creature_id>"],
    ["source", "axenstax"],
    ["source_version", "<mod version>"],
    ["element", "fire"],
    ["archetype", "hero"],
    ["range_class", "wanderer"],
    ["biome", "forest"],
    ["world_hash", "<sha256 of world_seed + world_name>"],
    ["verifier", "glimkin.world"]
  ],
  "content": "",
  "pubkey": "<player npub>",
  "created_at": <unix>,
  "sig": "<player signature>"
}
```

The schema is **identical** to what the Glikkin mobile app publishes for a physical glimpse, with only the `source` tag distinguishing. The federated verifier (D-012) accepts the event, runs the same validation pipeline, and increments the creature's witness count globally.

---

## 9. Federated Verifier integration

The AxeNStax mod is the first non-Glikkin-app consumer of the federated verifier. This validates the verifier-registry architecture (Glikkin doc `federated-verifier-brainstorm.md`, decision D-012).

### 9.1 Verifier trust model

Server admins configure which verifiers the AxeNStax server trusts:

```toml
# server.toml under [mods.glimkin]
verifiers = [
  "glimkin.world",            # canonical Decented verifier
  # "verifier.alice.example", # DIY verifier, post-MVP
]
```

Default: only the canonical verifier. Server admins can add community verifiers as they appear on Nostr's verifier-registry events.

### 9.2 Lookup flow

When a Portal Block is forged or manifests:

1. Server identifies the creature_id (either from genesis event for forged, or from a recent attuned-creatures feed for manifestations).
2. Server queries each configured verifier in order: `GET https://<verifier>/g/<creature_id>` → creature manifest (element, archetype, range_class, current resonance, art catalogue references).
3. First successful response is used; verifier identity is included in the journal event (`["verifier", "<host>"]`).
4. On glimpse, server POSTs the signed witness event to the same verifier (which relays it onward to Nostr).

### 9.3 Offline-mode degradation

Single-player AxeNStax worlds may have no network. The mod handles this gracefully:

- **No verifier reachable on world start:** disables ambient manifestation; allows Anchor and Vision events using a small pre-cached creature manifest set bundled with the mod.
- **Verifier becomes unreachable mid-session:** queues witness events locally; flushes them on next reconnect.
- **No network ever:** mod works fully offline against the cached MVP creature set; events are signed and stored locally; if/when the world is opened with network later, queued events flush.

This is brand-consistent — *"The other realm doesn't keep time the way we do. Whenever you return, something will be waiting."*

---

## 10. Range Class — the in-voxel rehide rule

A creature's Range Class is set at first glimpse (per `game-design.md` §5.4). It governs how far the portal must move to reopen.

### 10.1 Voxel distance calibration

AxeNStax player walking speed is 4.317 blocks/second (standard). At this speed:

| Range Class | Physical Glikkin | Voxel Glikkin Mod | In-world equivalent |
|---|---|---|---|
| Homebody | Max 500m from glimpse | Max 200 blocks | ≈ 45 seconds walk; same village |
| Wanderer | Min 1km, max 20km | Min 2000 blocks | ≈ 8 minutes walk; clearly a journey |
| Voyager | Min 5km, no max | Min 5000 blocks | ≈ 20 minutes walk; cross-biome travel |
| Pilgrim | Min 20km, no max | Min 20000 blocks | ≈ 80 minutes walk; cross-region pilgrimage |

These are deliberately *less than* the real-world equivalents (a 1:5 conversion factor) because voxel exploration is denser and players cover more interesting variety per metre.

### 10.2 The Portal Seed item

After a glimpse, the Portal Block goes inert. The player who hosted the manifestation (or who forged the portal) can break it — at which point it drops a **Portal Seed** item. The Portal Seed is the in-game equivalent of carrying the inert NFC card to a new location.

Portal Seeds:

- Have no recipe themselves (only obtained by breaking inert portals)
- Show their range class in the item tooltip
- Show their last-rehide-coords (a private bookmark, never published)
- Refuse placement closer than the range-class minimum from `last_rehide_coords`
- Refuse placement outside the element's biome-resonance set (with a 25% chance to allow off-resonance — Glikkin's same flexibility)

Portal Seeds **can** be traded between players face-to-face (same model as physical Glikkin card-passing). Trading a Portal Seed is the voxel equivalent of handing your friend a card. Reputation events fire when seeds change hands. **Portal Seeds CANNOT enter Vendor Block inventories** — this is hardcoded.

### 10.3 Pilgrim portals in single-player worlds

20000 blocks is roughly 312 chunks. On a single-player world this is a meaningful journey but not impossible (AxeNStax worlds are effectively unbounded). Pilgrim creatures are rare anyway (~10% of generated portals) and feeling like a true pilgrimage when one lands in your world is the point.

If a server's world is bounded (e.g., border = 10000 blocks per Spec 1 §9.2), Pilgrim creatures cannot rehide there at all — and the portal converts to a "Departed" state, with the creature having "moved beyond the reach of this world." This is brand-consistent and gives the player a satisfying narrative ending.

---

## 11. Forge mechanic — in-game

The Forge → Hide → Wait loop from `game-design.md` §4.2 ports almost verbatim.

### 11.1 Crafting a Portal Seed

The recipe (§7.3 above) requires the **Filament Fragment** material. Filament Fragments drop:

- 1 in 20 chance from any iron-ore or coal vein break (the more you mine, the more you accrue)
- 1 guaranteed from a Pilgrim-class creature glimpse (a small "the realm gave you something back")
- Cannot be crafted, traded, or sold

This makes forging earnable through normal AxeNStax play — but not trivial. A typical forge requires ~80 ore breaks.

The recipe also requires a **Nostr Signature**, which is a one-time UI event: when the player attempts the recipe at a crafting table, the game prompts them to sign a forge event using the engine signing bridge (Phase 4 prerequisite). The signed event becomes the creature's `genesis_event` on Nostr.

### 11.2 The genesis event

```jsonc
{
  "kind": 30902,                       // "glimkin-forge"
  "tags": [
    ["t", "forge"],
    ["realm_hint", "<world_hash>"],    // never the world seed; just a stable hash
    ["timestamp_anchor", "<unix>"]
  ],
  "content": "",
  "pubkey": "<player npub>",
  "sig": "<player signature>"
}
```

The federated verifier hashes the genesis event and derives the creature's element, archetype, and range class from it — identical algorithm to the physical-card path. The forger has no choice in what emerges. The realm decides.

### 11.3 The 48-hour attunement

The Portal Block exists, but is **dormant** for 48 in-world hours. AxeNStax's `/time speed` controls how fast this elapses in wall-clock time:

- At `/time speed 1`: 48 wall-clock hours (matches Glikkin physical exactly)
- At `/time speed 4` (alpha default): 12 wall-clock hours
- At `/time speed 64`: 45 wall-clock minutes

This gives server operators control over forge cadence. Brand-consistent: time in the other realm runs differently.

During attunement, the portal renders as a faint neutral pulse with no element colour. The forger cannot interact with it; they can't even confirm what's emerging. Mystery preserved.

### 11.4 Cannot self-glimpse

The first player on the server whose pubkey is **not** the forger's gets the first glimpse and establishes the genesis. The forger only sees their own portal "open" when this happens (a remote notification appears in their journal). If the world is single-player, the forged portal never opens — the player has to take a Portal Seed with them to a server where someone else plays, or wait for a friend to join.

This is exactly the physical Glikkin rule and it carries to multiplayer voxel naturally.

---

## 12. Cross-game witness amplification

This is the moment where a player's Glikkin journal lights up across realms.

### 12.1 The amplification

When you glimpse a creature via the AxeNStax mod, the witness event publishes to the same verifier the Glikkin mobile app reads. The same creature in your Glikkin app's journal will now show a new entry: "Witnessed via AxeNStax · <biome>, <date>". The creature's global clarity ticks up — sometimes meaningfully, if you're early in its witness curve.

### 12.2 Vision acceleration

Per `resonance-mechanic.md` §3 (referenced by `game-design.md` §5.6), witnessing a Vision that matches a creature already in your journal accelerates that creature's resonance growth. The voxel mod implements this:

- You have Blazehart (Fire/Hero) in your journal at 40% resonance.
- In AxeNStax you witness "The Forge Scar" — a Fire heartland vision.
- The verifier sees the vision event, sees Blazehart already in your journal, and applies the vision-acceleration bonus (per Glikkin's existing math).
- Next time you check the Glikkin app: Blazehart is at 45%.

Visions in AxeNStax are therefore *not* a parallel reward path — they slot directly into Glikkin's existing resonance economy. This is the cross-game integration at its most meaningful.

### 12.3 Co-witness events

When two AxeNStax players glimpse the same portal within 30 seconds of each other, both witness events carry a `["co_witness", "<other_pubkey>"]` tag. The verifier:

- Credits both as standard witnesses
- Adds a `+1 social bond` to both pubkeys' Glikkin reputation
- Publishes a kind-30903 "shared glimpse" event that appears on both players' Nostr timelines

This is the §4.3 "bring resonances together" mechanic, rendered in voxel.

---

## 13. Capability requirements (Spec 1 §8.2)

The mod's manifest. This is what server operators see before approving install.

```toml
[plugin]
name = "glimkin-mod"
version = "0.1.0"
authors = ["Decented", "Glimkin contributors"]
homepage = "https://glimkin.world"
manifest_signature = "<Decented Nostr signature over the build hash>"

[capabilities]
block_registry = true       # Portal Block × 6, Anchor Block, Filament Fragment
item_registry = true        # Portal Seed × 4 range classes
recipe_registry = true      # Portal Seed forge recipe
entity_spawn = true         # Transient creature billboard entity (no AI, no combat)
network_http = true         # Federated verifier API; allowlist enforced
nostr_sign = true           # NEW capability — signs witness + genesis events; requires engine signing bridge
nostr_publish = true        # NEW capability — publishes to configured relays
visual_overlay = true       # NEW capability — chunk-area atmospheric retexture (visions)
world_read = true           # Biome + light-level + Y inference for thinness
world_write = false         # MOD DOES NOT MODIFY BLOCKS — explicit refusal
player_inventory = false    # MOD DOES NOT TOUCH INVENTORIES — explicit refusal
filesystem = false          # No local writes

[network.http_allowlist]
hosts = ["glimkin.world", "*.glimkin.world"]  # plus admin-added verifiers

[network.nostr_relays]
default = ["wss://relay.trotters.cc", "wss://relay.glimkin.world"]

[limits]
memory_mb = 32
fuel_per_tick = 50000        # Half the default — mod is lightweight
http_requests_per_tick = 1
overlay_active_chunks = 4    # Hard cap on simultaneous vision overlays
```

The `world_write = false` and `player_inventory = false` lines are the audit story. A server admin reviewing this manifest sees that the mod cannot grief the world or steal inventory. That alone justifies the capability model.

Three new capabilities are introduced (`nostr_sign`, `nostr_publish`, `visual_overlay`) — all cross-game-generic and worth landing for other future mods.

---

## 14. Engine prerequisites

This mod cannot ship until the following AxeNStax foundation work lands:

### 14.1 WASM plugin runtime — minimal slice (NEW FOUNDATION SPEC NEEDED)

Spec 1 §8 is fully designed but unbuilt. The mod needs:

- `wasmtime` runtime integration in the server
- Capability-manifest parser
- Host functions: `register_block`, `register_item`, `register_recipe`, `spawn_entity`, `biome_at`, `world_read`, `log`, `http_request`, `nostr_sign`, `nostr_publish`, `visual_overlay`
- Plugin lifecycle: load, init, tick, shutdown, hot-reload
- Resource limits (memory, fuel, HTTP, overlay-chunks)
- WIT-bindgen interface generation

Estimated ~3000 LOC for the runtime slice covering this mod's needs. This is the largest single dependency.

### 14.2 Engine signing bridge

Per `2026-05-14-engine-signing-bridge.md` — engine-side NIP-46 client. Already a known blocker for Spec 1 Phase 4 and historically for Charter. The Glikkin mod is the third independent consumer of this bridge.

### 14.3 Biome worldgen (soft)

Per `farming-economy-long-run.md` — the mod uses heuristics for MVP but will benefit when worldgen lands.

### 14.4 Visual overlay primitive (NEW)

Chunk-area atmospheric overlay (fog, lighting, skybox tint) needs a renderer-level abstraction. Glikkin's visions are the first user, but the primitive is cross-game-generic — useful for weather or race effects in other games on the same primitives. Estimated ~400 LOC in the renderer.

---

## 15. Phasing

| Phase | What | Buildable solo? |
|-------|------|---|
| 0 | **Prerequisite:** WASM plugin runtime minimal slice (separate foundation spec) | Yes |
| 1 | **Prerequisite:** Engine signing bridge (`2026-05-14-engine-signing-bridge.md`) | Yes |
| 2 | **Prerequisite:** Visual overlay primitive in renderer | Yes |
| 3 | Schema crate — port `@glimkin/voxel-schema` types into shared Rust crate consumable by the mod and verifier | Yes |
| 4 | Portal Block (6 element variants) — placement, idle render, manifest format, signed-by-creator manifest | Yes |
| 5 | Veil-thinness module — per-chunk thinness calculation, threshold events, audio/particle pulses | Yes |
| 6 | Glimpse mechanic — server gate, HMAC roll, veil-holds branch, cinematic, journal write | Yes |
| 7 | Creature billboard sprite renderer — load Glikkin 2D art via verifier; 5-second walk animation | Yes |
| 8 | Range Class system + Portal Seed item — break-to-seed, rehide distance enforcement, trade gate | Yes |
| 9 | Forge mechanic — Filament Fragment drops, recipe, Nostr signing prompt, 48h attunement, no-self-glimpse | Yes |
| 10 | Federated verifier integration — `glimkin.world` HTTP API, attested manifest fetch, witness POST | Yes |
| 11 | Witness event publish — sign + broadcast kind-30900 to configured relays | Yes |
| 12 | Vision overlay — 5 visions for MVP, biome-and-element-triggered, atmospheric retexture | Yes |
| 13 | Vision-acceleration cross-game integration — verifier confirms acceleration of in-journal creature | Yes |
| 14 | Co-witness detection — simultaneous-glimpse identification, kind-30903 emission | Yes |
| 15 | Anchor Block — admin-placed portal-magnet block | Yes |
| 16 | Glimpse-Token bridge — accept short-TTL tokens from `glimkin.world/g`, queue physical-bridge manifestations | Yes |
| 17 | Journal UI in-game — read-only book showing player's Glikkin journal entries | Yes |
| 18 | Offline-mode degradation — local cache, event queueing, brand-voice error states | Yes |
| 19 | **Mode B — Journal fetch + bonded-creature spawn loop.** Verifier journal query, per-pubkey resident roster, in-world entity spawn for Forming+ creatures, particle-hint render for Faint creatures | Yes |
| 20 | **Mode B — Blur-by-resonance rendering ladder.** Per-band sprite opacity + halo + glow rules; live update when verifier reports resonance change | Yes |
| 21 | **Mode B — Per-element ambient behaviour.** Element-specific affinity/avoidance + idle wander patterns; gentle tempo enforced; observation-layer (no collision) | Yes |
| 22 | **Mode B — Non-witness interactions.** Walk-near notice, Communion event (30s presence), Offering block + ceremony, Legacy in-world fade | Yes |
| 23 | **Mode B — Visibility modes.** Per-player perceptual layer; `personal` / `shared` / `host` / `disabled` config; split-screen co-presence handling | Yes |
| 24 | Server admin config — `[mods.glimkin]` config section, verifier list, manifestation rates, visibility mode | Yes |
| 25 | Cross-mod isolation hardening — explicit refusal hooks in Vendor Block, crafting, anti-cheat, combat (Mode B creatures un-hittable) | Yes |
| 26 | Charter age-policy inheritance — guardian-flag-driven mode (under-13 sees glimpses + Mode B; forge HTTP gated by guardian) | Yes |
| 27 | Axolittle playtest gate — both Mode A discovery and Mode B personal-realm | No — playtest |

**Total:** 27 phases, ~7000 LOC mod-side + ~3500 LOC for the three engine prerequisites. Heavier than v1 of this spec because Mode B is a major addition, but Mode B's emotional payoff justifies the work and most of its infrastructure (verifier journal fetch, per-pubkey perceptual layer, ambient entity behaviour) is reusable across Decented games.

Phases 3–26 are solo-buildable in series. Phase 27 is the playtest gate.

**v0.2 (post-MVP):** Arena Block + voxel-as-theatre battles (§24). Additional ~1500 LOC. Deferred until Mode B has been playtested and Glimkin's combat-resolution API is stable as a remote dependency.

---

## 16. Brand alignment guardrails

These are non-negotiable. Every player-facing string and visual element must pass these tests.

### 16.1 Voice rules (per `brand-identity.md`)

| Do | Don't |
|---|---|
| "Something stirs on the other side. Are you ready to look?" | "New creature discovered! +1 XP!" |
| "Recorded. Your glimpse has been added to the journal." | "ACHIEVEMENT UNLOCKED" |
| "The veil held firm this time." | "Error: tap failed. Try again." |
| "The portal has closed, but the memory stays." | "Pokemon escaped!" |
| Silent journal entry | Toast notification with creature name |
| Subtle particle drift | Spinning loot box animation |

### 16.2 No FOMO mechanics

- No daily quests
- No time-limited creatures
- No "first witness" leaderboards
- No streak counters
- No "comeback" notifications

### 16.3 No achievement-style feedback

The glimpse itself is the reward. The journal entry is the record. Anything beyond that is brand drift.

### 16.4 Cross-mod isolation

Hard refusals (compile-time, not runtime):

- Portal Seeds cannot enter Vendor Block inventories
- Glikkin creatures cannot appear in crafting recipes
- The mod's HMAC and Proof-of-Play HMAC are separate primitives — Glikkin glimpses never pay sats
- The mod's audio never duplicates AxeNStax level-up sounds (banned palette)

---

## 17. Cross-game lift surface

Every primitive built for this mod should be reusable across the Decented platform. Specifically:

| Primitive | Reusable for |
|---|---|
| WASM plugin runtime | Every future AxeNStax mod |
| Engine signing bridge | Charter, Spec 1 Phase 4, every Nostr-aware mod |
| Visual overlay primitive | weather, race effects, anomalies in other games on the same primitives |
| Veil-thinness calculation | Any "atmospheric event" system; generalises to weather, mood, ambient encounters |
| Federated verifier client pattern | Any cross-game attestation use case |
| Glimkin schema crate | Direct reuse in other Decented games that want Glikkin journal entries |
| Co-witness Nostr event kind | Any cross-game shared-experience tagging |
| Range-class travel rule | Generalises to any "this thing must move" mechanic — quest hand-ins, vendor restocks |
| Forge-via-Nostr-signature recipe pattern | Any mechanic where a player ritual produces a content-addressed in-world artefact |

The plugin runtime alone justifies the engineering investment; the rest is bonus.

---

## 18. Risks and open questions

### 18.1 Engine prerequisites are heavy

Plugin runtime + signing bridge + overlay primitive total ~3500 LOC of foundation work. This is the largest engineering prerequisite tower we've ever required for a single feature. Mitigation: each prerequisite is independently valuable for other waves (Vendor Block, Charter Phase 1, future mods).

### 18.2 Glimkin-side coordination

The mod publishes to Glimkin's verifier and reads Glimkin's creature catalogue. This requires Glikkin to:

- Stand up the canonical verifier (in progress per `apps/verifier`)
- Define the cross-source event schema (currently single-source)
- Agree to the new kind numbers (30900, 30901, 30902, 30903)
- Bless the AxeNStax mod as an attested source

Mitigation: same team owns both products. Brief Glikkin maintainer (likely Staxolottle himself) before phase 3.

### 18.3 Biome detection accuracy

Current AxeNStax worldgen emits one biome; the heuristic mapping in §5.1 will be approximate until worldgen lands. Glimpses tagged with the "wrong" biome dilute Glimkin's biome-diversity stat math.

Mitigation: ship MVP with heuristics; flag the limitation in journal entries (`["biome_confidence", "heuristic"]` vs `["biome_confidence", "worldgen"]`). The verifier can weight heuristic-tagged biomes at 0.5× for diversity calculations.

### 18.4 Performance of Vision overlays

A chunk-area atmospheric retexture touches lighting, fog, skybox, and ambient audio. On WASM and low-tier GPUs this could stutter. The `overlay_active_chunks = 4` limit in the capability manifest is the hard ceiling.

Mitigation: pre-bake HDRI environment maps for MVP visions rather than real-time shader-based ones. Profile on Axolittle's hardware before phase 12 lands.

### 18.5 Brand drift risk

Every wave of mod features risks drifting the tone toward AxeNStax-default loudness. The fewer hands that touch the player-facing strings, the better. Brand voice should be locked in a fixtures file early (phase 3) and reviewed by Staxolottle before each subsequent phase.

### 18.6 What about kids with their own physical Glimkin cards?

The mod is gentle and works whether or not the player has any physical cards. The physical-bridge (§7.4) is a bonus for those who do. Charter age policy already governs Bitcoin and chat; no additional gating needed for Glimkin (which has no extractive economy).

### 18.7 What if Glimkin's combat/breeding/metamorphosis layers land before the mod ships?

The mod's MVP explicitly excludes those layers. If they ship Glimkin-side, the mod's journal UI can passively display them (read-only) without implementing them in-engine. No surface-area expansion for combat-in-voxel — that's a separate spec, deliberately deferred.

### 18.8 The "Filament Fragment" name

It's borrowed from `visions.md` (§25, "The Filament Garden" — Electricity vision). Same lore family, different in-game item. If this confuses, rename to **Portal Shard** or **Veilglass**. Defer until phase 4 implementation.

---

## 19. Out of scope for MVP

- **Voxel arenas / battle staging (§24).** Out of v0.1; in scope for v0.2 after Mode B playtest. The Vision-overlay primitive (§14.4) and Mode B spawn machinery (§23) built for v0.1 are exactly what arenas need, so v0.1 doesn't preclude them — just defers.
- **Battle simulation.** Even when arenas ship, AxeNStax never *simulates* Glimkin combat — it visualises Glimkin's resolution. Combat math stays in Glimkin's app/verifier; voxel is the stage.
- **Mating / breeding.** Requires the metamorphosis system. Future spec.
- **Metamorphosis.** Same.
- **Full 72-species roster.** MVP ships 9 (the original Glimkin MVP set). Element expansions (Air, Electricity, Ether) ship in subsequent waves.
- **Full 30 visions.** MVP ships 5. Remaining 25 ship in batches.
- **DIY verifier support.** Per D-012, the federated registry is designed for this, but MVP trusts only `glimkin.world`.
- **Bitcoin / sats payments.** Glikkin doesn't pay sats. The mod doesn't either. Hard rule.
- **Cross-game battles** (e.g., a sibling game vs AxeNStax glimpse-and-spar). Out of scope until both games' mod ecosystems are mature.
- **Selling Portal Seeds for sats.** Hard rule. Trading face-to-face only.
- **NFT-anything.** Hard rule.
- **Auto-discovery from physical card hidden in real-world near the player's IRL location.** Privacy and location-data violation; not on roadmap.

---

## 20. The pitch in one paragraph

The Glimkin mod is the smallest, sharpest, brand-safest mod we can build to prove three things at once: that AxeNStax's plugin runtime can be trusted with high-stakes content, that Decented is a real cross-game platform with a shared identity layer, and that a mod can change a game's voice as well as its mechanics. It is also Glimkin's digital onramp — kids who can't access physical cards can start a journal in AxeNStax that carries forward to physical play if and when they get cards. Neither product loses anything. Both gain. And the engine infrastructure built to land it (WASM runtime, Nostr signing bridge, atmospheric overlay primitive) unblocks every future mod in the queue.

---

## 21. Build authorisation

This document is design only. **No engine code is to be written from this spec until:**

1. Staxolottle has explicitly authorised the mod and its prerequisite stack
2. The "WASM plugin runtime — minimal slice" foundation spec has been written (separately) and authorised
3. The engine signing bridge foundation (`2026-05-14-engine-signing-bridge.md`) is queued or in-flight
4. Glikkin's verifier infrastructure has a confirmed canonical endpoint

Until then, this spec is the brief. It will be revised based on feedback from both Staxolottle and Axolittle before any code is written.

---

## 22. Deployment topology

A spec that doesn't say where the mod runs is incomplete. This section answers four operational questions: who runs the mod code, what clients need to participate, what worlds the mod is reachable in, and whether Decented should host a flagship Glimkin world.

### 22.1 Where the mod code runs

Per Spec 1 §8.1, plugins are server-side: they execute in `wasmtime` inside the AxeNStax server process. **Clients run no mod code.** Single-player is not an exception — the client's embedded `HostedServer` runs the mod in the same local process.

This has three useful consequences:

- **No client-side mod install.** Players join a Glimkin-modded server using a stock AxeNStax client. No version pinning, no "you need Glimkin Forge v0.3 to connect."
- **No mod-version drift between players.** Everyone on a given server sees identical behaviour because there's only one copy of the mod, on the server.
- **Server operators bear the trust decision.** A player joining a server is implicitly trusting its mod set; the engine surfaces installed mods + capabilities at connect time so the player can decide.

### 22.2 What the client needs

The mod uses three **engine-level capabilities** that must exist on every client to render the experience correctly:

1. **Asset push.** Server transmits textures, models, and audio for unknown block / item / entity IDs; client renders them as opaque data. No client-side code execution. (Generic — used by every future mod.)
2. **Visual overlay primitive.** Chunk-area fog/lighting/skybox/ambient-audio swap, driven by a server-issued overlay packet. Required for Visions. (Generic — useful to weather mods, race effects, environmental storytelling.)
3. **Book UI extension.** A read-only book page that fetches content from a configured URL (the federated verifier) and renders it within the existing in-game book UI. Required for the in-game journal display. (Generic — useful for any mod that needs to surface remote content.)

These three primitives **land in the engine itself**, not in the mod. They are gated by capability negotiation at connect time: a server using `visual_overlay` will refuse to accept clients whose engine doesn't support it. Pre-launch this is moot (all clients ship with the same engine version); post-launch it becomes a normal client/server version compatibility concern.

A consequence worth being clear about: **clients on engine versions older than the one that ships these primitives cannot join Glimkin-modded servers.** Server operators see this in the version-skew error; players see a friendly "this server needs a newer Axe'n'Stax client" message.

### 22.3 The four deployment layers

| Layer | Who runs the server | What it is | Brand control |
|---|---|---|---|
| **L1 — Flagship Glimkin World** | Decented | One canonical AxeNStax world, mod installed, hand-tuned Anchor placement, curated biome zones, vision rates dialled by the Glimkin team | Full |
| **L2 — Community servers** | Anyone | Any AxeNStax server admin installs the mod from the mod marketplace, picks their trusted-verifier set | None — variation expected |
| **L3 — LAN / private** | The host player | Mod runs in the host's HostedServer; co-witness works between LAN players | None |
| **L4 — Single-player** | The player | HostedServer runs the mod against a local cache + queued events; forge works but cannot self-glimpse | None |

**Invariant across all four layers:** witness events sign with the player's pubkey and publish to the federated verifier; the verifier accepts them on identical terms regardless of which layer sourced them. A creature glimpsed on a kid's solo world counts toward its global clarity exactly like a glimpse on the flagship server, exactly like a tap of a physical NFC card on a park bench. The realm doesn't care which window you looked through.

### 22.4 Mapping to Glimkin's federated trust model

The four-layer model is **the same structural decision** Glimkin already made for physical cards (per `federated-verifier-brainstorm.md` D-012). The mapping is exact:

| Glimkin physical layer | Glimkin mod layer | Trust signal |
|---|---|---|
| **Genesis Drop / Keeper Kit** (Decented-programmed cards) | L1 Flagship Glimkin World | "Verified by Decented" |
| **Found in the wild** (someone else's card) | L2 Community server you joined | "Verified by <server's chosen verifier>" |
| **DIY** (blank NFC + own master key) | L2-as-DIY: a server admin running their own verifier post-MVP | "Verified by <community verifier>" |
| **Self-forge** (own card, own forge) | L4 Single-player forge | Cannot self-glimpse; awaits another witness |

This means the federated-verifier-registry design (D-012) carries directly into the voxel mod. The PWA still serves as the canonical UI for the journal across all paths; the AxeNStax server is one more attested source that registers with a verifier and publishes signed events.

### 22.5 Should Glimkin host a flagship world?

**Yes, AND.** Not yes-instead.

**Why flagship is essential:**

- **It's the demo.** "Want to see Glimkin in voxel?" → one URL, brand-controlled experience, no prior knowledge required.
- **Biome-worldgen compensation.** AxeNStax worldgen emits a single biome in practice today. The flagship world can hand-place biome-declaration markers and Anchor Blocks so the experience showcases all 15 Glimkin biomes correctly. Community servers will be patchier until worldgen lands.
- **Anchors first impressions.** New players don't know which community servers are tasteful. Flagship is the safe entry.
- **Matches Glimkin's Genesis Drop tier exactly** — the canonical, trusted-by-default channel.

**Why flagship-only would be wrong:**

- It kills the "Decented is a platform" thesis we use everywhere else.
- It contradicts the federated-verifier decision (whose whole point was *not* having one canonical infrastructure path).
- It removes the audit-story value of the capability manifest — if only Decented runs the mod, nobody else needs to review it.
- It blocks viral spread through friend-of-friend servers, which is where voxel communities actually grow.
- It would make the mod look like Decented gatekeeping, which contradicts Glimkin's "anti-NFT, anti-marketplace, open source" stance.

**The right framing:** the flagship world is Decented's *contribution* to the network, not the network's *only point of access*. Same way Decented's canonical verifier is one of many possible verifiers, not the only one.

### 22.6 Realistic ordering

The flagship world depends on AxeNStax launch infrastructure existing — and per project memory (`project_alpha_launch_posture`), `axenstax.app` is not live; Decented isn't running an AxeNStax launch server yet. So:

```
1. Plugin runtime + signing bridge + overlay primitive land in the engine
2. AxeNStax goes live (independent track)
3. Glimkin mod ships against the engine
4. (in any order, possibly interleaved:)
   4a. Decented stands up the flagship Glimkin world on AxeNStax launch infra
   4b. Community servers install the mod
   4c. Players play it in single-player / LAN
```

Steps 4a–4c are **independent**. Community servers (4b) and solo play (4c) can ship before the flagship world (4a) exists. The verifier accepts witness events from any of them on the same terms. This means the mod has real-world utility from day one, not "soon when Decented launches a server."

### 22.7 Server-config schema

The `[mods.glimkin]` section of `server.toml` covers all four deployment layers (the differences are mostly which values operators choose, not which fields exist):

```toml
[mods.glimkin]
enabled = true
forge_enabled = true                 # disable to make the server manifestation-only
manifestation_rate_multiplier = 1.0  # 0.0 = no ambient portals; flagship may tune higher
vision_rate_multiplier = 1.0         # same shape for visions
anchor_density_cap = 1               # anchors per 256 chunks; flagship may raise

[mods.glimkin.verifiers]
trusted = ["glimkin.world"]          # canonical only by default
# DIYer additions land here post-MVP

[mods.glimkin.relays]
publish = ["wss://relay.trotters.cc", "wss://relay.glimkin.world"]
read = ["wss://relay.trotters.cc", "wss://relay.glimkin.world"]

[mods.glimkin.tone]
voice_pack = "canonical"             # locks Glimkin brand strings; alternatives forbidden in v0.1
```

The `voice_pack` lock is intentional — brand integrity is non-negotiable. v0.1 ships exactly one voice pack and refuses overrides. Future packs require Glimkin-team review.

### 22.8 Capacity and economics

Per-player runtime overhead of the mod on the server:

- Federated verifier HTTP requests: ≤ 1 per glimpse, cached aggressively; well within Spec 1's 1/tick capability cap
- Nostr event publish: ≤ 1 per glimpse, fire-and-forget with retry queue
- Per-chunk thinness calculation: integer math, O(loaded chunks); negligible
- Portal block state: ≤ 100 bytes per block; bounded by `anchor_density_cap`
- Vision overlay: client-rendered; server cost is the overlay packet (~200 bytes)

A 100-player server running the mod adds well under 1% CPU and negligible memory over baseline AxeNStax. **There's no economic barrier for community servers to run it** — which matters, because the federated model depends on small operators being able to participate without infrastructure cost.

Bandwidth from the federated verifier is the only real shared-cost surface. Decented operates the canonical verifier; if community servers proxy through it heavily, that's a Decented cost. Mitigation: aggressive client-side cache (verifier responses are mostly immutable creature manifests); rate-limit per-server-pubkey.

---

## 23. Mode B — The Personal Realm

This section is what §1.3 introduces as the gaze inverted: bonded creatures crossing the veil *into* your voxel realm. It is at least equal in weight to the discovery sections (§3–§12), and arguably the emotional centre of the mod. Discovery recruits; the personal realm is what keeps players coming back.

### 23.1 The core claim

The AxeNStax world is, in lore, "another realm" — a block-built reality with its own physics, sitting alongside the real world and alongside Glimkin's other-realm. A creature you have bonded with through a glimpse — physical card, Mode A portal, or otherwise — has a thread of resonance that connects you across realms. Mode B lets that thread *manifest visually*: the creature appears in your voxel world as an ambient inhabitant. Not summoned. Not commanded. Just *present*, because you are.

You can see them like you'd see a cow. You can walk past them. You can stand near and watch. You cannot fight them, capture them, or trade them — they're not game objects in that sense. They're the visible form of an existing bond.

### 23.2 Manifestation rules

On entering an AxeNStax world with the mod enabled and your Signet identity authenticated:

1. Mod queries the configured federated verifier for your journal: `GET /g/journal/<your_pubkey>` → list of (creature_id, resonance, source list, current biome affinity).
2. For each creature at resonance ≥ 10% (Forming and above), the mod schedules a passive ambient entity spawn within your loaded chunks. Spawn location is biased by the creature's element-biome resonance (§5.3).
3. Creatures below 10% (Faint) manifest only as **particle hints** — element-tinted wisps that drift through the player's vicinity occasionally. You can sense them. They sharpen into visible sprites as resonance grows.
4. Total simultaneous ambient population is capped per loaded chunk-set (default 8; admin-configurable). Larger journals **rotate** through residents — different creatures present on different sessions, weighted toward the most-recently-witnessed and the highest-resonance bonds.
5. Creatures are persistent across world saves and re-spawns: the same Crysthorn that wandered the player's forest yesterday will be there today, with continuity of position preserved in mod state.
6. Creatures **do not collide** with the player physically — they're observation-layer entities. The player can walk through them. (Otherwise you'd be constantly bumping into your own bonded companions.)

### 23.3 The blur-by-resonance ladder

Glimkin renders the same Mature image at different clarities depending on the player's resonance band (`resonance-mechanic.md` §9). The mod inherits this verbatim — and uses it as the **resonance-becomes-visible** loop that connects in-app bond-building to in-world experience:

| Band | Resonance | Voxel render |
|---|---|---|
| Faint | 0–10% | Translucent particle wisps only; no sprite. You feel them; you don't see them. |
| Forming | 10–30% | Heavily blurred billboard sprite, ~30% opacity, element-tinted halo |
| Clear | 30–70% | Recognisable but soft sprite, ~60% opacity |
| Vivid | 70–95% | Fully resolved, fully opaque sprite |
| Radiant | 95–100% | Sprite plus a slow elemental glow and trailing particles |

The cross-app feedback loop is the load-bearing thing: a player who deepens a bond in the Glimkin app (by being witnessed by others, by gaining biome diversity, by witnessing matching Visions) *sees their voxel companion become clearer*. The world rewards the relationship.

### 23.4 Per-element behaviour

Bonded creatures wander based on their element's nature. The movement uses AxeNStax mob-AI primitives but with gentler tempo than any hostile mob — these are companions, not encounters.

| Element | Affinity | Avoidance | Idle behaviour |
|---|---|---|---|
| Fire | Sunlight, campfires, lava, warm biomes | Water, rain | Slow drift; pauses to "warm" near heat sources |
| Water | Water blocks, rain, beach edges | Deserts, fire | Floats just above water; submerges briefly; reappears |
| Earth | Stone, caves, forests, tree bases | Open sky, fast movement | Slow walks; long pauses sitting still |
| Air | High Y, open sky, mountaintops | Tight spaces, deep caves | Floats; rides invisible thermals; never lands |
| Electricity | Daytime, exposed plateaus, hilltops | Underground, wet biomes | Erratic short hops between perches |
| Ether | Twilight, ruins, biome boundaries, doorways, dawn/dusk | Direct sun on open plains | Fades in/out; appears at the periphery; rarely centred |

These behaviours are intentionally **subtle**. Bonded creatures are *present*, not *attention-seeking*. They don't follow the player. They don't beg for input. They live in the world like everything else does — you see them when you look.

### 23.5 Non-witness interactions

The mod lets the player engage with bonded creatures without generating new witness events (because the veil only parts once per pubkey per creature). Interactions are emotional and ceremonial, not mechanical:

| Interaction | What happens | Nostr event |
|---|---|---|
| Walk near (≤ 8 blocks) | Creature notices, briefly turns to face the player | None |
| Approach (≤ 2 blocks) | Subtle element-particle aura blooms; soft ambient sound from the element plays | None |
| Stand with for 30 in-world seconds | A "Communion" event — counts as a Vision-acceleration-equivalent toward this creature's resonance growth (anti-grind cap: one Communion per creature per 24 in-world hours) | kind-30904 |
| Hit with weapon | **Refused at the engine level.** The damage hook rejects the source; the swing passes through harmlessly. No HUD message. | None |
| Place an "Offering" block nearby | Cosmetic only. The Offering block is craftable, holds one item, and emits a soft elemental signal. The act publishes a "memory" Nostr event readable in the Glimkin journal app as a moment-record. | kind-30905 |
| Ratify Legacy | When a bonded creature is eligible for Legacy (per `game-design.md` §5.6) and is present in the world, the player can perform the in-world Legacy ceremony — the creature fades in a slow elemental dissolution; a memorial entry is written to the journal. | kind-30906 |

The **Communion** event is the load-bearing one. Time spent in the bond is gently rewarded with accelerating resonance growth. It is *not* a grind: the per-day cap and the resonance-curve diminishing returns prevent grinding. But it does mean that players who spend time with their companions are subtly rewarded over players who don't.

The **Offering** mechanic is the playful one — there is no game-mechanical effect, but the act of laying something at a creature's preferred spot becomes a moment recorded in the journal. Years later, a player can scroll back and see "On 2027-03-14, I laid a wheatsheaf for my Sporecap in the forest behind the village." This is a small, sentimental feature that costs almost nothing to ship and pays back enormous emotional dividends.

### 23.6 Visibility modes — who sees whose creatures

When multiple players share a world, whose journal manifests visually is a meaningful privacy decision. Server-config:

| Mode | Behaviour | Use case |
|---|---|---|
| `personal` (default) | Each player sees only their own bonded creatures; the perceptual layer is per-player | Privacy-first; no journal exposure between strangers |
| `shared` | All players in the world see all players' bonded creatures simultaneously | Family households, trusted LAN, shared discovery |
| `host` | Only the world-host's creatures manifest; visitors see the host's bonded set | Curated showcase worlds; host's journal seeds the realm |
| `disabled` | Mode B off entirely; only Mode A discovery is active | Servers focused on AxeNStax's native gameplay |

`personal` is the default because it mirrors Glimkin's privacy-first stance — a journal is yours, and rendering it visibly to strangers is opt-in.

For **split-screen households** (AxeNStax supports 4-player local co-op): each splitscreen player's perceptual layer is independent. Two siblings on the same TV each see their own bonded creatures in their own viewport on the same world. In `shared` mode they see each other's too — which is the family-bonding-moment use case (showing your child your Blazehart by both being in `shared` together).

### 23.7 The Legacy fade

When a creature in your journal reaches Legacy, it gradually fades from your voxel realm over 7 in-world days. Each day its opacity drops 14%. On the final day, a single soft chime plays when the player is nearest the creature. Then it is gone. The journal entry remains as memorial.

This is the brand-voice closing of a bond, ported intact: *"The other realm doesn't keep time the way we do. Whenever you return, something will be waiting"* — except, for Legacy, it isn't waiting any more, and the world is quieter for it.

### 23.8 What Mode B does *not* do

Hard guardrails. Same spirit as §16:

- Bonded creatures cannot be commanded, ridden, leashed, fed-for-effect, or trained.
- They generate no loot, no XP, no sats.
- They cannot be killed; they cannot kill anything.
- They cannot enter the player's inventory, the Vendor Block, crafting recipes, or any economic surface.
- They cannot be photographed/screenshotted for AxeNStax achievement-style sharing surfaces. (Players can take screenshots of the world; the screenshot includes the creature; we cannot prevent this, nor should we. But the game does not actively prompt or reward sharing.)

### 23.9 Implications for the four deployment layers

Mode B significantly shifts the centre of gravity from §22's framing:

- **L4 (Single-player)** rises in importance. A solo world becomes the most natural Mode B venue — your private companion realm. Glimkin's privacy-first ethos says this is the *right* default for most players.
- **L1 (Flagship)** stays important for discovery (Mode A); Mode B can also be enabled there but defaults to `personal` visibility.
- **L2 (Community)** runs whatever mix the operator chooses; a community could legitimately run Mode-B-only as a "companion realm" server.
- **L3 (LAN)** becomes the family-household sweet spot — `shared` mode plus split-screen means everyone's bonded creatures roam together.

This makes the platform thesis stronger, not weaker. Every player's personal world becomes a venue. The flagship is one of many places to be — and frequently not the place a player spends most of their time. That's healthy: it means the platform isn't a centralised attention sink.

---

## 24. The Voxel Arena — battle as theatre

This section turns Glimkin's existing battle system into a staged voxel experience. v0.1 scope-flag: arenas land in v0.2, after Mode B has been playtested. This section is the design commitment so v0.1 doesn't accidentally box arenas out.

### 24.1 The opportunity

Glimkin's battle system (`game-design.md` §6) is designed to be channel-agnostic — two witnesses bring their resonances together "in person, over Nostr, or through any messaging app you both share — the creatures interact across the veil, and you see the result as a shared vision." Battle is, in Glimkin's framing, *a shared vision* rather than a real-time simulation.

The voxel world is exceptionally well-suited to render that shared vision. Not as a combat simulator — as a *theatre*. Mechanics stay Glimkin's; staging is AxeNStax's. The voxel game becomes the stadium.

### 24.2 The Arena Block

A craftable (or admin-placed) **Arena Block** marks a battle-eligible site. The duel sequence:

1. Two players "sit at" the Arena Block from opposite sides.
2. Each player has at least one bonded creature at Vivid or Radiant resonance — confirmation overlay checks this.
3. Both confirm the match.
4. The mod requests a shared Vision from Glimkin's verifier — the same vision-resolution algorithm Glimkin already uses to pick a battle's stage. Result is one of the 30 Glimkin Visions, picked by element-matchup affinity.
5. The arena's surrounding 32×32 chunk area temporarily retextures into that Vision (reuses the §6 + §22.2 visual-overlay primitive).
6. Both bonded creature sprites appear in the arena, facing each other.
7. Players take turns picking Strike / Guard / Feint moves through a chat-overlay-style UI; the moves resolve per Glimkin's existing combat math (element interactions, stats, archetype bonuses).
8. Voxel rendering animates the creatures per move — but **the engine does not simulate combat**; it visualises Glimkin's resolution. AxeNStax is the renderer; Glimkin is the rules.
9. Outcome is signed and published to the verifier per Glimkin's existing combat-event schema.
10. After the match the Vision fades; both creatures return to their owners' Mode B presence in their respective worlds.

### 24.3 What this is not

- **Not a real-time fighting game.** Turn-based, faithful to Glimkin's combat tempo. Players have time to read the move list and think.
- **Not loot-generating.** No items drop, no sats change hands inside AxeNStax, no in-game rankings.
- **Not creature-injuring.** Bonded creatures cannot "die" in voxel; on KO they fade out gracefully and re-appear in their owner's Mode B world afterwards. The voxel arena is a *story* the creatures and witnesses share; it does not deplete the bond.
- **Not cross-realm corruption.** The creatures here are the *same* creatures from each player's journal; nothing about the duel mutates their identity or stats outside Glimkin's existing rules.

### 24.4 Spectators

Other players in the arena's vicinity can watch from spectator stands (placed by the world-builder). They see the same retextured Vision overlay and the same creature animations. Per AxeNStax Spec 6 §10 (Spectator Economy), tip jars and broadcast hooks can attach.

This is a clean bridge to AxeNStax's spectator economy without entangling Glimkin in payments: **spectators tip the broadcaster, not the combatants.** Glimkin's combat is unchanged by the presence of money flowing around it; the broadcast is an AxeNStax-side feature happening adjacent to a Glimkin-side game.

### 24.5 Cross-game lift

The Arena Block primitive — designate a venue, request a shared Vision from a federated verifier, retexture the area, run a turn-based exchange — is engine-generic.

- A sibling game could have a kitchen-themed dueling block where chefs cook against each other to Glimkin's rhythm.
- Another sibling game could have racing-themed arenas where mounts race through a Vision-overlaid track.
- AxeNStax itself could host non-Glimkin arenas using the same primitive for other player-vs-player ceremonies.

Each Decented game runs its own theatrical interpretation; the underlying combat-resolution math stays Glimkin's.

### 24.6 Scope flag

Arenas are **out of v0.1 scope** (see §19). v0.2 ships them once Mode B is playtested by Axolittle and Glimkin's combat-resolution API is stable enough to be a remote dependency. The v0.1 build does not preclude them — the Vision overlay primitive (§14.4) and the Mode B creature spawn machinery (§23) are exactly the infrastructure arenas need.

### 24.7 Where battles fit in the player journey

A player's Glimkin arc, end to end across both products:

1. **Witness** a creature (via NFC card, Mode A portal, etc.) → resonance forms.
2. **Deepen** the bond via Glimkin app (witnesses by others, biome diversity, Visions) → resonance grows.
3. **Live alongside** the creature in your AxeNStax world (Mode B) → Communions accelerate growth.
4. **Battle** another player's resonance in a voxel arena (v0.2) → shared vision, no loss to the bond.
5. **Mate / Metamorph** the creature when conditions are met (Glimkin-app; out of mod scope).
6. **Ratify Legacy** in your AxeNStax world when the creature is ready → final fade, memorial sealed.

Each step in this arc has a home — Glimkin app or AxeNStax mod — and the journal stitches them together. The two products are not competing for the player's time; they're *taking turns being the right surface* for each phase of the relationship.

---

## Appendix A — Open design questions

1. **Should AxeNStax show the player's full Glikkin journal in-game, or just the AxeNStax-sourced entries?** Showing the full journal is more cohesive but pulls more data and surfaces creatures the player only knows via physical cards. Lean: full journal, read-only, with a filter toggle.

2. **What happens when a creature reaches Legacy while its portal is active in an AxeNStax world?** Lean: the portal block fades to grey over 24 in-world hours and emits a final "memorial" vision before becoming inert permanently.

3. **Should server admins be able to disable forge?** Yes — `forge_enabled = false` in the config. Some servers will want manifestation-only.

4. **Should we expose a server-wide "thinness boost" admin command for events?** Yes — `/glimkin thinness <0..1> <duration>` for special events. Brand-controlled.

5. **Should the mod publish anything to AxeNStax's own activity feeds?** No. Glikkin events live on the Glikkin verifier. AxeNStax-side leaderboards / activity feeds must not show them. This is brand isolation.

---

*End of foundation spec.*
