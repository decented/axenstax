# Nostrich Racing

**Tagline:** Race ostriches. Collect Bitcoin. Lay eggs on your friends.

## Concept

Emu-racing-meets-Mario-Kart on Nostriches. Every rider sits on their own Nostrich — the purple ostrich mascot of the Nostr ecosystem. The bird has a mind of its own. The rider is just along for the ride.

The tone is slapstick physical comedy — never aggressive, always funny. The Nostrich pecks, kicks, and lays eggs because that's what ostriches do. The rider hangs on and hopes for the best.

## Visual Style

- **Mario-inspired** — clean, rounded, colourful characters that read instantly at speed
- **Nostrich + riders** — smooth low-poly models, bold silhouettes, toybox feel
- **Tracks** — voxel-built environments (block grid for terrain, props, shortcuts)
- **Particles everywhere** — dust clouds, feathers, sparks, splashes, egg splats

The characters are NOT blocky cubes. Smooth meshes for Nostriches and riders, voxel grid for track environments. The contrast makes both pop.

## The Nostrich

The star of the game. One base Nostrich model, customisable per player:

- Feather colours / patterns
- Saddle and accessories (Lightning bolt saddle, relay antenna hat, etc.)
- Personality quirks (head bob frequency, idle animations)

### Movement Personality

| Action | Animation | Feel |
|--------|-----------|------|
| Sprint | Neck forward, wings tucked, dust trail | Fast and committed |
| Drift | Legs scrambling sideways, tail feathers fan for balance | Chaotic, overcorrection |
| Jump | Wings spread wide, glide across gaps | Graceful for one second |
| Brake | Feet skidding, head bobbing back | Comical panic stop |
| Hit | Stumble, shake off like a wet dog | Slapstick recovery |
| Idle | Peck at ground, look around confused | The bird is bored |

Ostriches don't corner well. They burst in straight lines and flail in turns. That's not a bug — it's the core feel. Think emu racing energy: bouncy, chaotic, slightly ridiculous, genuinely fun to watch.

## Riders

The rider is the player's identity. Differentiation comes from the rider, not the mount.

- **Nostr identity** — your npub pulls your profile or a voxel avatar onto the bird
- **Cosmetics** — helmets, goggles, capes, colours
- **The rider reacts** — leans into turns, ducks on boosts, wobbles on hits, celebrates on overtakes

The rider never controls attacks. The Nostrich acts on its own. The rider is just hanging on.

## Items & Collectibles

### Track Collectibles

| Item | What it does |
|------|-------------|
| **Bitcoin** | The coins of the track. Line the racing line, fill your purse. On Lightning-enabled servers, could map to real sats |
| **Keys** | Unlock shortcut gates, hidden routes, bonus track areas. Nostr-flavoured — like unlocking relays |

### Offensive Items

| Item | What it does |
|------|-------------|
| **Egg drop** | Lay an egg behind you. Next rider hits it and spins out. Can stack a clutch of three |
| **Egg launch** | Lob an egg forward in an arc. Satisfying splat on impact |
| **Kick** | Auto-triggers on close overtake. Back legs flick out, nudges opponent sideways |
| **Wing slap** | Close-range side swipe when neck-and-neck. Knocks them into a wobble |
| **Tail fan** | Dust cloud behind you, briefly reduces visibility for the follower |

### Defensive / Utility Items

| Item | What it does |
|------|-------------|
| **Turbo seed** | Nostrich eats it, eyes go wide, burst of speed |
| **Lightning bolt** | Zap the leader — fits the Lightning Network theme |
| **Feather storm** | Screen fills with feathers for nearby riders, visibility chaos |
| **Nest** | Deploy a temporary blockade on the track, riders detour around it |
| **Wrench** | Hazard on track — hit one and your Nostrich stumbles, shakes it off |

### Design Rule

Every item is either something an ostrich naturally does (egg, peck, kick, dust) or something from the Nostr/Bitcoin world (lightning, keys, bitcoin). No weapons. No violence. Just birds being birds and Bitcoin being Bitcoin.

## Track Design

Tracks are voxel-built — same block placement system as the main Axe'n'Stax engine.

### Track Builder

- Players build tracks using the existing block placement tools
- Publish tracks to Nostr relays for anyone to race on
- Fork and remix other people's tracks
- Rate, comment, and zap tracks via Nostr

### Track Features

- **Shortcut gates** — locked doors opened by collecting keys
- **Terrain variety** — dirt (dust clouds), stone (sparks), water (splashes), ice (sliding)
- **Ramps and jumps** — wings-out glide sections
- **Hazard zones** — wrenches, mud patches, narrow cliff edges
- **Boost pads** — feathers glow, Nostrich accelerates

## Multiplayer & Identity

- **Nostr npub = racer identity** — profile, stats, track creations all tied to your key
- **Race results posted to relays** — leaderboards are Nostr events
- **Zap integration** — zap a track creator, bet on a race, tip the winner
- **Spectator mode** — watch races live, comment via Nostr

## Bitcoin Integration

Inherits the Axe'n'Stax payment architecture:

- **Track Bitcoin** — collectible coins on the racing line, optionally backed by real sats
- **Race stakes** — optional entry fee, winner takes pot (Lightning micropayments)
- **Track creator revenue** — popular tracks earn zaps and play fees
- **Cosmetic marketplace** — rider/Nostrich skins purchasable with sats
- **Platform never touches funds** — same noncustodial policy as main game

## Technical Notes

### What the engine already provides

- Voxel chunk rendering (tracks)
- Block placement system (track builder)
- Input handling with gamepad support (racing controls)
- Networking via QUIC (multiplayer races)
- Entity rendering (Nostrich + rider models)
- Particle systems (dust, feathers, sparks)

### What's new for racing

- Vehicle physics (acceleration, drift, collision)
- Smooth mesh loading for characters (non-voxel models)
- Race logic (laps, positions, item distribution, finish)
- Track metadata format (start grid, checkpoints, item spawn points)
- Camera system tuned for racing (follow cam, speed effects)

### Shared Infrastructure

Nostrich Racing and Axe'n'Stax share the same engine, same networking, same Nostr identity, same Lightning integration. A player's account works in both games. A world server could host a mining zone AND a race track.

## The Pitch

> You're on an ostrich. It doesn't really listen to you. There are eggs everywhere. Someone just zapped you with lightning and your bird is shaking it off. You cross the finish line sideways, feathers flying, and earn 50 sats. You have no idea what happened but you want to go again.
