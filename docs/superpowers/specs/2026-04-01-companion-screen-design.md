# Companion Screen Design Spec

**Date**: 2026-04-01
**Status**: Design
**Phase**: 3 (Gameplay Depth)
**Research**: `docs/research/2026-04-01-companion-screen-rg35xx.md`

## Overview

A budget handheld gaming device (Anbernic RG35XX, ~£40) acts as both a game controller and a companion display. The player holds the handheld and plays on a TV. The handheld sends button/stick input to the game host over WiFi and receives game state to render on its 3.5" screen. No touchscreen — physical buttons only.

## Hardware Target

| Feature | Spec |
|---------|------|
| Device | Anbernic RG35XX (Plus/H for built-in WiFi) |
| Price | ~£40 |
| Screen | 3.5" non-touch, 640x480 |
| Controls | D-pad, 2 analog sticks, ABXY, 4 shoulders (L1/R1/L2/R2), Start/Select |
| OS | Linux (custom firmware: muOS, GarlicOS, Batocera) |
| WiFi | Built-in (Plus/H models), USB dongle (original) |
| USB-C | Charging only (not used for data — see Security) |
| Battery | ~3000mAh (5-8 hours) |

## Connection Architecture

```
┌──────────────────────────┐           ┌─────────────────┐
│ N100 / Pi 5 (game host)  │           │ RG35XX          │
│                          │   WiFi    │                 │
│  Game Server ◄───────────┼───TCP────►│ Companion App   │
│  (port 7710)             │           │                 │
│                          │           │ Sends: buttons  │
│  Heartwood (separate     │           │ Recv: game state│
│  process, no companion   │           │                 │
│  access)                 │           │ USB-C = power   │
└──────────────────────────┘           └─────────────────┘
```

### WiFi (data connection)
- Companion connects to same network as game host (home WiFi)
- Or: game host runs its own WiFi hotspot (car, LAN party, mobile hotspot with client isolation)
- TCP socket on dedicated companion port (7710)
- Application-level protocol — no OS trust, no device driver interaction
- If WiFi drops, companion screen goes blank, controller input stops (same as gamepad disconnect — player freezes, game continues)

### USB-C (power only)
- Charges the handheld while playing
- Does NOT carry game data
- Plug in or pull out anytime — WiFi connection is unaffected
- No OS-level trust relationship — eliminates BadUSB attack vector
- Critical because game host may run Heartwood with family key tree

### Security Model
- Companion talks to game server only, never to Heartwood
- WiFi is application-level: game server validates all input, drops bad packets
- No filesystem access, no kernel-level trust, no device enumeration
- Compromised companion can only send bad button presses — server ignores them
- Heartwood is process-isolated, Tor-only, separate security boundary

## Protocol

### Companion → Host (input)

Sent at 20Hz (matching game tick rate):

```rust
/// Companion input packet — same fields as PlayerIntent
struct CompanionInput {
    /// Analog stick values
    move_forward: f32,   // -1.0 to 1.0
    move_right: f32,     // -1.0 to 1.0
    look_dx: f32,        // radians
    look_dy: f32,        // radians
    /// Button states (bitpacked)
    buttons: u16,        // sprint, sneak, jump, break, place, inventory, pause, etc.
    /// Companion-specific
    active_tab: u8,      // 0=inventory, 1=map, 2=crafting, 3=scanner
    companion_action: u8, // 0=none, 1=sort_inventory, 2=craft_item, etc.
    action_param: u16,   // item/recipe index for companion actions
}
```

Size: ~20 bytes per packet. At 20Hz = ~400 bytes/sec.

### Host → Companion (game state)

Sent at 4Hz (companion display doesn't need 60fps):

```rust
/// Game state subset for companion display
struct CompanionState {
    /// Inventory (always sent)
    inventory: [SlotData; 36],   // item_id: u16, count: u8 per slot
    hotbar_slot: u8,
    
    /// Tool info
    held_tool_durability: f32,   // 0.0-1.0, or -1 if no tool
    held_tool_name_idx: u16,     // index into string table
    
    /// Player state
    health: u8,
    position: [f32; 3],
    facing_yaw: f32,
    
    /// Active tab data (only the currently viewed tab)
    tab_data: TabData,
}

enum TabData {
    Inventory,                    // no extra data needed (inventory already above)
    Map(MapData),                 // explored chunks bitmap, player markers
    Crafting(CraftingData),       // available recipes, ingredient counts
    Scanner(ScannerData),         // cross-section block data below player
}
```

Size varies by tab:
- Inventory: ~120 bytes (36 slots × 3 bytes + overhead)
- Map: ~500-2000 bytes (compressed explored region)
- Crafting: ~200-500 bytes (available recipes + counts)
- Scanner: ~256 bytes (16x16 cross-section of block IDs)

At 4Hz = ~2-8 KB/sec total. Trivial for WiFi.

## Companion App (runs on RG35XX)

### Architecture

Lightweight Rust binary cross-compiled for ARM (RG35XX is ARM-based). Renders directly to framebuffer or via minifb/softbuffer — no GPU required on the companion.

```
┌────────────────────────────────────────┐
│ Companion App                          │
│                                        │
│  ┌──────────┐  ┌────────────────────┐  │
│  │ Network  │  │ Renderer           │  │
│  │ Thread   │  │ (framebuffer/CPU)  │  │
│  │          │  │                    │  │
│  │ TCP sock │  │ Tab views:         │  │
│  │ Send inp │  │  - Inventory grid  │  │
│  │ Recv st  │  │  - Map 2D          │  │
│  └──────────┘  │  - Crafting list   │  │
│                │  - Scanner slice   │  │
│  ┌──────────┐  └────────────────────┘  │
│  │ Input    │                          │
│  │ Thread   │  ┌────────────────────┐  │
│  │          │  │ State              │  │
│  │ evdev/   │  │ Last CompanionState│  │
│  │ gilrs    │  │ Active tab         │  │
│  └──────────┘  └────────────────────┘  │
└────────────────────────────────────────┘
```

### Tab Navigation

| Button | Action |
|--------|--------|
| L1 | Switch to Inventory tab |
| R1 | Switch to Map tab |
| L2 | Switch to Crafting tab |
| R2 | Switch to Scanner tab |

Tab switches are instant (local state change). The active tab is sent to the host so it knows which TabData to include in the next CompanionState.

### Per-Tab Controls

**Inventory (L1):**
| Control | Action |
|---------|--------|
| D-pad | Move cursor between slots |
| A | Grab/swap item at cursor |
| B | Drop item |
| Y | Auto-sort inventory |
| X | Quick-move selected item to/from hotbar |
| Start | (reserved for game pause) |

**Map (R1):**
| Control | Action |
|---------|--------|
| Left stick | Pan map |
| Right stick | Zoom in/out |
| A | Drop waypoint marker |
| B | Clear nearest marker |

**Crafting (L2):**
| Control | Action |
|---------|--------|
| D-pad | Browse recipe list |
| A | Craft selected recipe (if materials available and near table) |
| B | Back to category list |
| L1/R1 | Switch category (while in crafting tab) |

**Scanner (R2):**
| Control | Action |
|---------|--------|
| D-pad up/down | Scroll depth layers |
| A | Mark ore position (adds waypoint on map) |

### Rendering

The companion renders at 640x480 using CPU-based 2D rendering:
- Inventory: coloured rectangles for slots, simple text for item names/counts
- Map: 2D tile grid with biome colours, player arrow
- Crafting: scrollable text list with item icons (pre-rendered sprites)
- Scanner: colour-coded grid (16x16), one colour per block type

No 3D rendering. No GPU. No wgpu. The companion is a 2D information display.

## Game Host Integration

### Transport

The companion connects through the existing Transport trait:

```rust
pub trait ClientTransport: Send {
    fn send_to_server(&self, data: &[u8]);
    fn try_recv_from_server(&self) -> Option<Packet>;
}
```

A new `TcpCompanionTransport` implements this for WiFi TCP connections. The game server accepts companion connections on port 7710 (separate from game client connections on port 7700).

### Companion as Player

From the game server's perspective, a companion is a player with extra capabilities:
- It sends PlayerIntent (same as any controller)
- It additionally sends companion actions (sort, craft, tab switch)
- It receives a CompanionState subset instead of full world chunks
- It does NOT receive chunk mesh data (it doesn't render 3D)

### ScreenContent Integration

```rust
pub enum ScreenContent {
    LocalPlayer(usize),
    // Phase 3:
    Companion { player_index: usize, connection: CompanionConnection },
}
```

When a companion connects, the game server:
1. Authenticates it (simple token exchange or Nostr sig in Phase 6)
2. Creates or associates a PlayerSlot
3. Begins sending CompanionState at 4Hz
4. Begins accepting CompanionInput at 20Hz

### Companion + Controller Overlap

A companion IS a controller. When connected, it replaces or supplements the player's gamepad:
- If player was using a standalone gamepad: companion replaces it (handheld becomes the controller)
- If split-screen with two companions: each companion is one player's controller + display
- The game doesn't need a separate gamepad AND companion — the companion has all the buttons

## Discovery

### Home Network
- Game host broadcasts presence via mDNS: `_axenstax._tcp.local`
- Companion app scans for this service on startup
- Shows list of found game hosts with world names
- Player taps A to connect

### Host Hotspot (car/LAN)
- Game host runs WiFi hotspot with SSID like `AxeNStax-XXXX`
- Companion connects to this WiFi (configured once, remembers)
- mDNS works on the hotspot network too
- Or: companion connects to hardcoded IP (10.0.0.1 on hotspot)

## Multi-Companion

Two kids, two companions, one TV:
- Each companion authenticates as a different player
- Each sees their own inventory/map/scanner
- TV shows split-screen (or one player full-screen)
- Or: no TV at all — each kid plays on their companion screen only (handheld mode, reduced world view)

## Phasing

| Step | What | Dependencies |
|------|------|-------------|
| 1 | CompanionInput/CompanionState protocol definition | None |
| 2 | TcpCompanionTransport (host side: accept connections, send/recv) | LAN co-op transport (Phase 2) |
| 3 | Companion app skeleton (connect, send buttons, receive state) | Cross-compilation for ARM |
| 4 | Inventory tab rendering on companion | Inventory system maturity |
| 5 | Map tab | World exploration tracking |
| 6 | Crafting tab | Expanded crafting system |
| 7 | Scanner tab | Ore/block data query API |
| 8 | mDNS discovery | LAN discovery (Phase 2) |

Steps 1-3 can start as soon as LAN co-op transport exists (Phase 2). Steps 4-7 develop alongside gameplay depth (Phase 3). Step 8 shares infrastructure with LAN server discovery.

## What We Don't Build

- No touchscreen support (RG35XX doesn't have one)
- No 3D rendering on companion (CPU 2D only)
- No companion-to-companion communication (each talks to host only)
- No companion as standalone game client (it always needs a host)
- No web-based companion (native app on Linux only for now)
