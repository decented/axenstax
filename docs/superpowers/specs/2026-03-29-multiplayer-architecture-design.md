# Multiplayer Architecture — Split Screen, Host-as-Server, Dedicated Server

**Date:** 2026-03-29
**Status:** Specifying

## Vision

All three multiplayer modes share one architecture: a server-authoritative game simulation connected to 1-4 client renderers via a transport abstraction.

| Mode | Server location | Clients | Transport |
|------|----------------|---------|-----------|
| Split screen (2-4) | In-process thread | 2-4 in same process | mpsc channels |
| Host-as-server | In-process thread | 1 local (channel) + N remote (network) | Mixed |
| Dedicated server | Separate binary | All remote (network) | Network only |

## Core Architecture

### Server

The `GameServer` owns all authoritative state:
- Block world (chunks, water, leaf decay)
- Entity world (hecs ECS — mobs, dropped items)
- Player registry (positions, health, inventory per player)
- World time, mob spawning
- Physics simulation (20 TPS tick loop)

The server:
1. Receives `ClientInput` packets from all connected clients
2. Runs the tick simulation
3. Sends `ServerUpdate` packets to each client

### Client

Each `GameClient` owns:
- Camera (position, yaw, pitch — derived from its player's server-side position)
- Renderer (GPU pipeline, mesh cache)
- Input state (keyboard/mouse/gamepad)
- Chunk mesh cache (rebuilt when server sends chunk updates)
- HUD state (hotbar, hearts, crafting UI)

The client:
1. Captures input, sends `ClientInput` to server
2. Receives `ServerUpdate`, applies to local state
3. Renders the world from its camera's perspective

### Transport Abstraction

```rust
trait Transport: Send {
    fn send(&self, packet: &[u8]);
    fn try_recv(&self) -> Option<Vec<u8>>;
}
```

Implementations:
- `ChannelTransport` — wraps `std::sync::mpsc` for in-process communication (zero-copy, zero-latency)
- `NetworkTransport` — wraps UDP socket for LAN/online (future Phase 2)

### Packet Types

```
ClientInput:
  - player_id: u32
  - forward/backward/left/right/jump/sneak/sprint: bool
  - yaw, pitch: f32
  - left_click, right_click: bool
  - hotbar_slot: u8

ServerUpdate:
  - player_states: Vec<PlayerState>  // position, health, held item
  - chunk_updates: Vec<ChunkUpdate>  // block changes
  - entity_updates: Vec<EntityDelta> // mob positions, health, spawns, despawns
  - world_time: u32
```

## Split Screen Rendering

For 2-4 players on one screen:

| Players | Layout |
|---------|--------|
| 1 | Full screen |
| 2 | Top/bottom halves |
| 3 | Top half + bottom-left/right quarters |
| 4 | Four quarters |

Each player gets their own:
- Viewport (scissor rect)
- Camera uniform
- Render pass (same world geometry, different view)

Chunk meshes are shared (one GPU upload, rendered multiple times from different cameras). Entity meshes rebuilt per-frame per-camera (different cull distances).

### Input Routing (Split Screen)

- Player 1: Keyboard (WASD) + Mouse
- Player 2: Gamepad (if available) or arrow keys + numpad
- Players 3-4: Additional gamepads

For the prototype: Player 1 = keyboard+mouse, Player 2 = gamepad OR second keyboard mapping.

## Phase 1: Split Screen (In-Process)

**Scope**: 2-player split screen, shared world, top/bottom split.

**What changes**:
1. Extract world simulation from GameState into `GameServer` struct
2. Create `GameClient` struct per player (camera, input, HUD state)
3. Server runs in the main tick loop (not a separate thread yet)
4. Render the world twice per frame (two viewports)
5. Route Player 1 input from keyboard/mouse, Player 2 from gamepad or second key mapping
6. Both players share the same chunk meshes on GPU

**What stays the same**:
- Block world, chunks, meshing — unchanged
- Entity system, mob AI, combat — unchanged (just runs for 2 players)
- Crafting, inventory — per-player instances
- Save/load — saves all player states

**New UI**: "Split Screen" button in main menu. Second player joins with gamepad button press.

## Phase 2: Host-as-Server (LAN)

**Scope**: Host clicks "Open to LAN", server thread starts, remote clients connect.

**What changes**:
1. Move server to a dedicated thread with its own tick loop
2. Add `NetworkTransport` using UDP (quinn/QUIC crate for reliability)
3. Local client communicates via channel, remote clients via network
4. Chunk data serialized and sent to remote clients on connect
5. "Open to LAN" button in pause menu
6. Server advertises on local network (mDNS or broadcast)

## Phase 3: Dedicated Server

**Scope**: Server as standalone binary, all clients remote.

**What changes**:
1. `axenstax-server` binary that runs GameServer with no renderer
2. Config file for world, port, max players
3. `axenstax-engine` binary becomes client-only when connecting to remote
4. Same protocol, same transport — just no in-process client

## Not In Scope (this spec)

- Client-side prediction / interpolation (Phase 2+)
- Anti-cheat (Phase 3)
- Matchmaking / Agones (Phase 3+)
- Voice chat
- Player skins / models (players rendered as coloured boxes first, like mobs were)
