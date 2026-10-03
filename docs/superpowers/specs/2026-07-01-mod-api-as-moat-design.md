# Mod API as the Moat — LLM-native modding, and keeping the core unsplintered

**Date:** 2026-07-01
**Status:** Design note (strategy + architecture direction). Not a build spec yet.
**Owner call captured:** Staxolottle. Concern is *coherence*, not stopping forks —
an open-source project can't stop forking, and forking-as-freedom is a marketing
asset we lean into. The job is to make **modding** the natural low-friction path
so the core stays coherent and unsplintered while forking stays a celebrated
right.

---

## The thesis

The historical limiter on Minecraft-style creativity was never desire or ideas —
it was **coding ability** (age, skill, or inclination). LLM-assisted coding is
vanishing that limiter: a kid can point an LLM at the game and have it write the
extension. That is a once-in-a-platform shift. AxeNStax's differentiator over
Minecraft is not merely "open source = freedom to fork" — it's **designed for the
freedom to mod with your LLM**. Some mods will one-shot; some will need a
Cursor/Claude Code session. Either way the barrier is gone.

The risk that rides in with that opportunity is **fragmentation**. If the easiest
thing for an LLM to do is fork the whole engine, the ecosystem splinters and the
core loses coherence. So the design goal is to make the **modded path** the
lowest-friction, best-documented, most-connected path — and let forking remain a
right people rarely need to exercise.

**Fork-vs-mod is won by two things: a boundary, and where the value sits.**

---

## Mechanism 1 — The boundary (technical): a first-class, versioned mod API on a capability sandbox

- **Elevate WASM plugins (Spec 01) to *the* product surface.** A WASM mod can
  extend anything but cannot splinter core: it targets a stable ABI, runs
  sandboxed with an explicit capability list, and ships as a signed artifact.
- **The reason to fork disappears when the API can do what people want.** API
  instability and capability gaps are the *only* things that force otherwise-happy
  ecosystems to splinter (see Minecraft lesson below). So the coherence job is
  two disciplines:
  1. **Stable API** — semver it, don't break it. Mods keep working across core
     updates, so modders stay in the fold instead of pinning to a fork.
  2. **Generous API** — when a mod needs something core doesn't expose, the
     healthy path is **upstream the capability into the API**, not fork. Provide a
     fast lane for capability requests / upstreaming.
- **The mod boundary is also the safety boundary.** A mod that switches on
  communication still passes through the age-gate capability (Signet age
  assurance). Coherence and safeguarding-by-design are enforced at the *same*
  seam — you don't get a splinter *or* an un-gated comms channel by writing a mod.
  (This ties back to the KIDS Act / regulated-service analysis: the plugin
  boundary is where "safe defaults, sensitive features age-gated" is mechanically
  guaranteed rather than disclaimed.)

## Mechanism 2 — The gravity (social/economic): put all the network value on the platform side of the boundary

A mod keeps the whole platform; a fork becomes an island. Make that asymmetry
literal:

- **Multiplayer over Nostr**, mod discovery/registry, identity/Stash, live
  updates, the economy rails, the operator market — all sit on the *platform* side
  of the plugin boundary.
- **A mod plugs into all of it. A fork forfeits all of it.** Nostr-native, signed
  mod distribution makes this concrete: publish a mod as a signed event and it's
  in the network; fork the engine and you've left the network.
- This asymmetry — **not a license, not lockdown** — is what keeps the core
  unsplintered. It is a gravity well, not a fence.

---

## The Minecraft lesson (this is the opening)

Minecraft fragmented — Bukkit → Spigot → Paper, Forge, Fabric, and eventually
full clean-room reimplementations — **because Mojang shipped no official mod API
for years.** Modders had to hack an obfuscated core, so they forked and
reimplemented. **Ship the API first and you don't inherit that fate.** This is
both the moat and the coherence mechanism at once.

## "Freedom to mod with your LLM" — the doc surface that makes it real

An LLM only one-shots cleanly against a **small, stable, documented surface**:

- Machine-readable API description (`llms.txt` / structured API doc).
- Mod **manifest schema** + explicit **capability list**.
- Fillable **scaffolds / templates** an LLM drops code into.
- Worked examples spanning "one-shot in the browser" up to "Claude Code session".

Design so the *modded* path is what one-shots — forking the whole engine is
*harder* for an LLM than filling a plugin template. **The friction gradient does
the governance for you.**

## The canonical registry — a Schelling point, not a lock

- A **Nostr-native, npub-signed mod registry**, curated/rated. This is exactly
  where the Charter **curator-quorum Web-of-Trust ratings** idea earns its keep.
- "Official" is not enforced by license — it is **earned** by being where the
  trust marks, the updates, and the network are.

---

## Positioning

Lean into forking-as-freedom in the marketing. Engineer so the low-friction,
LLM-native, network-connected, above-board path is always **modding**. When
Minecraft locks features down and shady MUDs spring up, AxeNStax is the version
parents, operators, and distribution channels *trust* — legitimacy is the moat,
and the LLM-native mod surface is the feature Minecraft structurally cannot match
(obfuscated Java, no official API, redistribution-hostile).

## Distribution (low-friction, neutral)

Distribute the neutral open-source software anywhere the community wants —
GitHub, a Nostr-native code host, torrent. Distribution of genuinely dual-use
software is protected; the regulated-service risk is about *operating* the
service, which we don't (multiplayer over Nostr, self-hosted Docker). **Do not**
pair distribution with induce-the-bypass framing (a "dev mode" or disclaimer
that's really a how-to) — that's the one pattern that converts a neutral tool into
an *induced* one (Grokster). Safe features stay age-gated at the capability
boundary instead.

---

## Open questions / next steps

- **Name & scope the mod API surface** (which subsystems get first-class,
  stable extension points first — blocks/items, entities/AI, scenarios/challenges,
  UI, economy hooks?).
- **Manifest + capability model** — what capabilities exist, which are
  age-gated, which need operator consent.
- **Registry mechanics** — Nostr kinds for mod publish/rating; how curation and
  the Charter WoT quorum plug in.
- **LLM doc-gen pipeline** — keep `llms.txt`/schema/scaffolds regenerated from
  the real API (same "regen-on-change" discipline as the ai-guide corpus) so the
  LLM surface never drifts from the code.
- **Upstreaming lane** — the process by which a mod's needed capability becomes a
  core API addition rather than a fork trigger.

## Related

- Spec 01 (Engine Architecture) — WASM plugins.
- `docs/superpowers/specs/2026-06-23-challenge-engine-vision.md` — ScenarioDef/
  ChallengeDef data-driven surface (a natural early mod target).
- KIDS Act / regulated-service analysis (this thread) — the plugin boundary as
  the safeguarding seam; age-gated-unlock vs induced-bypass.
- Charter curator-quorum WoT ratings — the registry curation layer.
