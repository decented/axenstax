# ADR-001: Full Custom Engine Build

**Status**: Accepted
**Date**: 2026-03-03

## Context

After extensive research into existing open-source voxel engines (Luanti, Veloren, Terasology, Cuberite, ClassiCube), we evaluated three strategic paths:

1. **Fastest-to-market**: Build on Luanti as a game + modpack
2. **Long-term scalable with minimal rewrites**: Luanti with a platform abstraction layer
3. **Custom-engine path**: Server-first platform with proven cloud-native primitives

## Decision

**Full custom build. Go big or go home.**

Axe'n'Stax is a moonshot project. The vision — a World Runtime Platform that scales from a single sleeping sandbox to 10k+ player celebrity events with integrated Bitcoin economics — cannot be constrained by another engine's architecture, assumptions, or community direction.

## Rationale

### Why not Luanti (or any existing engine)

- **CCU ceiling**: Luanti tops out at ~50-100 CCU per shard with tuning. We need architecture that doesn't have someone else's ceiling as our ceiling.
- **Networking model**: UDP on port 30000 with Luanti's protocol. We need full control over protocol design, packet efficiency, and bandwidth optimisation.
- **Mod system limitations**: Lua sandboxing is fine for community mods, not for building a payment-integrated, anti-cheat-hardened platform core.
- **Renderer constraints**: We need a resolution-agnostic, performance-tuned renderer — not one designed for general-purpose modding.
- **Distributed simulation**: Region-based world simulation across multiple workers requires engine-level architecture, not a mod bolted onto a single-server engine.
- **Bitcoin integration depth**: Payment flows, reward mechanics, and custody need to be first-class engine concerns, not HTTP calls from a mod.

### Why custom is the right risk

- **AI-driven development** dramatically reduces the engineering lift that historically made custom engines impractical for small teams.
- **Server-first design** means we architect for horizontal scale from day one instead of retrofitting it.
- **No licensing constraints** — we own every line, choose our licence, and never inherit LGPL obligations or community governance friction.
- **The research is done** — both deep research reports provide the technical foundation, cost models, and architecture patterns to build confidently.

## Consequences

- Higher initial engineering effort (mitigated by AI-assisted development)
- No existing mod ecosystem to leverage (but we define our own plugin architecture)
- Must build client and server from scratch (but we control the full stack)
- Full ownership of performance characteristics, protocol design, and scaling behaviour
- Research reports remain valuable as reference material for architecture patterns, cost modelling, and operational planning

## Technology Direction

- **Language**: To be decided (Rust, C++, or Zig for engine; capability-based plugin system)
- **Networking**: Custom protocol, UDP-first, designed for bandwidth efficiency
- **Orchestration**: Kubernetes + Agones for game server fleet management
- **Matchmaking**: Open Match or custom session directory
- **Payments**: LNbits integration as a platform service, not a mod
- **Persistence**: Pluggable storage backends, designed for cloud-native snapshots
