# ADR-002: Core Tech Stack

**Status**: Accepted
**Date**: 2026-03-03

## Decisions

### Engine Language: Rust

- Memory safe, high performance, zero-cost abstractions
- Compiles to WASM natively — required for web client
- `wgpu` crate abstracts WebGPU/Vulkan/Metal/DX12 behind one API — write once, ship to browser and desktop
- Strong game dev ecosystem (wgpu, winit, glam, rapier)
- AI-assisted development mitigates learning curve for the team

### Client Target: Native-First, WASM-Aware

- Native desktop client is the primary build target during development (Vulkan/Metal/DX12)
- Web client (WASM + WebGPU) comes second — after core gameplay loop works natively
- Engine code stays WASM-compatible by design: use `wgpu`, avoid platform-specific APIs
- No WASM-specific debugging tax during early development — move 2-3x faster
- When ready, same Rust codebase compiles to WASM with no migration needed
- `wgpu` makes this possible without a renderer abstraction layer

### Bitcoin Model: Hybrid

- The game works as a free voxel sandbox without Bitcoin (personal worlds, creative mode, survival)
- Bitcoin-enabled servers are the flagship experience (mine-to-earn, creator economies, pay-to-play)
- Two modes, one engine — server configuration determines whether Bitcoin features are active
- De-risks the project: great game even without Bitcoin
- Widens adoption: players come for the game, discover the economy
- Reduces regulatory exposure: Bitcoin is opt-in, not mandatory

### Self-Hosting: Two Tiers

- **Personal tier**: Single binary, zero config. A 12-year-old can run a server for friends on a laptop. No Bitcoin, no cloud, just the game.
- **Production tier**: Docker/K8s deployment with Bitcoin integration, Agones orchestration, autoscaling, and platform features.
- Same engine binary in both cases. Deployment mode and config determine capabilities.

### Networking: Existing Crates First

- Start with proven Rust networking libraries (e.g. `quinn` for QUIC) rather than a fully custom protocol from day one
- Go custom only when voxel-specific needs (delta compression, chunk streaming) exceed what existing crates provide
- WebTransport compatibility considered when web client target becomes active
- Revisit this decision when implementing Spec 04 (Networking)

### Audio: Deferred

- No audio library chosen yet — `kira` and `rodio` are the leading Rust options
- Decision deferred until gameplay loop is running and sound design begins
- Not a blocker for early prototyping

## Consequences

- Rust is the single language for engine (client + server)
- Platform services (matchmaking, payments orchestration, admin) may use other languages where appropriate
- All rendering goes through `wgpu` — no OpenGL fallback, no custom abstraction
- The engine must cleanly separate "core game" from "Bitcoin features" at the architecture level
- Single-binary mode must not require Docker, K8s, or any external services
- Native desktop is the only compile target until core gameplay is proven — WASM build comes later
- Networking starts with existing crate(s), custom protocol is a future optimisation
