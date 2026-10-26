# Axe'n'Stax Roadmap

*As of 2026-10-26 — source v0.2.27, network protocol v95, save format 59. The repository is public and the sites deploy from `main`.*
This file is **the** roadmap: where the project is, what is waiting, and what comes later.
For "what do we build next", see `docs/foundations/README.md` (the build queue); feature requests and ideas enter through the internal feature backlog.

---

## Where we are

Axe'n'Stax is an open-source, self-hostable voxel sandbox with its own engine, client and server, written in Rust. The build queue is empty: everything that was ready to build has been built. What remains is work only the owner can do (cutting the native release, and live tests with real devices and people), plus a short list of design decisions that block building further on top of the current foundations.

- **Version:** v0.2.27 in source, network protocol v95, save format 59. The published desktop installer is still v0.2.18 until the release below is cut, so what is described here is well ahead of what players can download.
- **Web (`play.axenstax.com`)** is an anonymous, local-only sandbox: a *taster*. No login, no multiplayer, no cloud save and no feedback channel on web. Worlds live in the browser and can be exported to the desktop app.
- **Native (Linux AppImage)** is the full game: Signet sign-in, LAN play, online play by contact, the dedicated server, and the feedback mailbox. Windows and macOS are not published as installers yet (build from source).
- **Android** is ported onto main (the APK builds in CI, `android` input off by default, and a tapped `axenstax:` link opens the game), but nothing is published: release signing is owner-side, and updates will come through zap.store, not an in-app updater.
- **Gamepad support** is built and working, but is parked as a launch priority.
- **Posture:** self-hosted, sovereign, player-owned. Worlds, saves and identity belong to the player; AxeNStax ships software, not a service. See the red lines at the end.

---

## Shipped

### Engine and rendering
- Custom Rust engine on wgpu: native desktop and WebAssembly/WebGPU (Chromium) from one codebase.
- Greedy meshing, texture-array atlas, day/night cycle, weather, water and lava flow, lighting, particles, snow layers, per-species mob tint, see-through glass and leaves, optional mipmaps.
- Graphics settings: a device-aware first-run preset, a pixel budget, and and an auto-quality governor that adjusts quality to hold the frame rate (it never shortens a joined client's view distance).
- Resolution-agnostic texture packs: disk packs with a manifest, a picker, and hot-swap without a restart.
- Headless test harness that boots the full client without a display; a single regression gate (`check.sh`) covering lint, tests, the WASM build and a bundle-size limit.

### World and gameplay
- Survival and creative modes, difficulty selector, and a one-way World Integrity Ledger (pure survival / ever creative / cheats used).
- Tool progression, smelting, crafting (2x2 and 3x3) with a recipe book, repair, armour, hunger and health, death drops and respawn.
- World generation with biomes, caves, ravines, mineshafts, villages, brigand hideouts and coal, iron, diamond, copper, magnesium and nitre ores (no tin, gold or amethyst source yet, so Bronze is unreachable in survival).
- Terrain generator v3 for new worlds: continents, deep oceans, rivers, mountains and cliffs, in a world now 256 blocks tall. v3 is frozen as shipped, and each world records its generator, so older worlds keep their terrain unchanged. New worlds also start on dry land with headroom. Terrain generator v4 (v3 plus dry caves) is built but unreleased: no world is stamped with it yet.
- Farming, fishing, composting, weather, water-driven and wind-driven machines.
- Electricity: wires, switches, batteries, lamps, windmill, water wheel.
- Proof of Play: every pickaxe strike runs an HMAC-SHA256 hash that is shown to the player as an educational proof-of-work primitive. It also drives optional rare drops. It is not a payout mechanism.
- Trials: timed races and explorer challenges, with an authoring guide and a lint that fails loudly on mistakes.
- A guided onboarding companion (Satoshi) that is fun-first and optional.
- Slash commands (`/time`, `/gamemode`, `/tp`, `/give`, `/we`, `/place` and more) and in-game chat.

### Creatures
- A broad mob roster: hostile, passive, tameable pets, steeds, bees and hives, and a racing animal (Nostrich).
- Animal genetics and breeding, animal products (eggs, milk, shearing), taming and pet beds.
- Server-side mob AI, spawning and entity physics, with deltas broadcast to joiners.

### Building and creative tools
- Block shapes: stairs, slabs, vertical slabs, doors (including double), panes, walls, fence gates, signs, item frames.
- Building variants: slabs, stairs, doors, trapdoors, fence gates and walls in every wood and stone.
- Middle-click pick block: the block under the crosshair lands in your hand, in creative, survival and on a joined server.
- Schematics and blueprints with a build guide, a Drafting Stamp, and world-edit tools.
- The Workshop: reskin and reshape blocks and mobs in game (paint, carve, symmetry, per-block wardrobe) and a Rig Studio for animated assets.
- Skin editor with Minecraft skin import and export, classic and slim arms, and painter tools.
- Exhibits (images on walls and standing frames) and a kiosk mode.
- Rails with auto-connecting bends, 45-degree ascending and descending ramps, and flat floor cables; Rail Freight phase 1, plus parts of phases 2 and 3: cart hull tiers (wood, iron, diamond), bulk vendors restocking from an adjacent depot chest, and a transit-only robbery record (`hostile_acts`).
- Cameras: opt-in third-person view (F5).
- Take-your-worlds-to-native: the web lobby exports one profile file, and the desktop app imports it (name clashes keep both).

### Multiplayer and identity
- Native "Host Game" and its joins run on Fray (a room code resolved on the player's own relays, plus a LAN beacon heard on that network only), with the Signet join challenge unchanged. Split-screen on one machine, and a headless dedicated server (Docker image and an Operator Console with allowlist, blocklist and require-sign-in).
- A `--keep-world` mode: the engine runs one persistent world for a home Keep (a separate program) over stdin and stdout. Joining a Keep by room code or an `axenstax://keep` link (first-time confirm) saves it to My Keeps, which the Join Game card lists, three newest first with a More Keeps button.
- The host lends its one world and ECS to its embedded server every tick (D1), so there is no second copy of the world; joiners are diffed from the host's real world.
- Joiners see, hurt and act on the server's mobs (D2a, D2b: attack and kill credit, feed, tame, shear, milk, Leads and pet commands). Riding works through server-accepted mounts (D2c-1), and shared steed packs have an implemented path (D2c-2) that still waits for an owner session.
- Late joiners are sent the world itself: the server pushes the columns players have touched, and a joiner regenerates untouched columns from the seed (B2a, B2b).
- The server runs every joiner's hunger, eating and sleep (C2a), and keeps its own copy of each joiner's inventory, saved per npub on the operator's machine (C3). The accepted part of C3d (typed actions with receipts and recovery) is in; its Strict enforcement mode is internally selectable only and parked, so the default Mirror mode still believes some of what a joiner claims.
- World handover: a host who needs to leave can hand the running world to another player in the session ("Hand this world to ..."), and everyone reconnects there; the original host keeps their own copy.
- Known limits, from `CLAUDE.md`'s known-debt list: a joiner's possession is only partly enforced (a placement check that is log-only, and the held item it claims for attacks, interactions and mined tools is believed, as is some of what it pays with, up to a bound); a dedicated server runs no species AI, breeding or Leads, so it refuses a joiner's feed, tame, Lead and pet command; villager quests for joiners are not finished (the D2c-3 foundation is merged, live delivery is not accepted); a `--no-lend` host (one release only) keeps a second copy of the world.
- The dedicated server ticks hives, crops, dispensers, pistons, furnaces, composters, kegs and hoppers (`simulates_block_machines`; on a lending host the host's own client ticks them instead), and a tick's changes are split across size-bounded StateUpdates. `CLAUDE.md`'s known-debt entry that says otherwise predates this.
- Online play by contact (native): join a friend's home-hosted world by their Signet persona npub or a bearer invite. Nostr relays carry connection setup only; the connection is then direct. Strangers get silence. Relays never carry game traffic.
- Server authority over remote players (movement, reach and block edits are validated; the server keeps its own copy of a joiner's inventory); join handshake with Signet-signed identity and the Fray join binding (v95).
- World chat (native), tiered by contact relationship, with a ceiling that can only be tightened.
- Signet sign-in by QR (mySignet pairing) and Signet contacts sync: Kin, Kith and blocks feed the address book, and a block removes a player from a running host.
- Optional, self-published Nostr announce for an operator's own server. Nothing is ever announced automatically.
- Death-drop items, and the server's mobs, carts and projectiles drawn on joiners from a changed-only entity diff.

### Sites and distribution
- Six public sites: game, docs, marketing, wiki (player guide), learn (lessons) and project home.
- Linux AppImage with an in-place updater (checks the site and a Nostr release feed), and a version indicator in the lobby.
- Cross-platform packaging pipeline and a third-party licence pipeline.
- Player documentation: wiki, learn journey, tutorials, operator guides.

### Safety, privacy and compliance hardening
- Web build has no login, cookies, analytics or feedback channel; the site apps do not log IP addresses.
- Feedback is native-only: per-report burner key, end-to-end encrypted, no replies, with an in-game status board.
- Crash-safe saves with damage quarantine, a staged data-directory migration, and a hardened updater.
- Money words are guarded by permanent tests on every text surface; Proof of Play never appears as an earning mechanic.
- Public repository housekeeping: licence, security policy, contributing guide and a credits file are in place.

---

## Now — waiting on the owner

Everything here needs a person, a second machine or a real device, or a decision that is not the code's to make.

**Live tests (cannot be verified solo)**
- Two-machine LAN play with a real remote signer: native to native, and native to dedicated server. (The web build has no sign-in, so there is no browser-plus-phone leg.)
- Online play by contact across two real houses (UPnP/STUN punch, bearer invite, persona attestation).
- Death drops, late-joiner chunk push and tool pickup, walked over by a second player, and a joiner hurting and riding the server's mobs.
- Signet contacts sync on a phone: pairing, Kin/Kith appearing, a block kicking a connected player, disconnect. This now also tests the relay change: pairing runs over the first of "Your relays" (a public relay by default), not the project's own relay.
- Native QR sign-in on a phone with mySignet. This now also tests the relay change: the QR advertises the first three of "Your relays" (public by default), and editing the list from the sign-in screen rebuilds the QR.
- A `/bug` report reaching the project's public inbox relays and being read back by the feedback reader, and the release event republished to the public relays so the in-app updater (which now reads the player's relays) finds it.
- Playtest of the unverified trials, and the electricity switch test across two machines.
- First-launch data-directory migration, the updater, and the current framing (v95, Host Game on Fray) on real hardware.

**Fun test**
- Play-testing with voluntary children who are not family. Family play has been a bug-hunt, not evidence that the game is fun, so fun is unvalidated.

**Public launch — done 2026-10-03/04**
- The repository is public as a fresh single-commit snapshot (old history archived privately), with fork-PR approval, secret scanning and push protection, private vulnerability reporting and a protected `main`.
- The privacy page is signed off by the owner (2026-10-04): controller published as *decented*, bug reports kept until dealt with and at most 6 months.
- The project's own relay is no longer a default anywhere in the native app or its tools (pending the phone tests above). Sign-in, contacts pairing, server discovery and the updater use the player's one editable "Your relays" list; feedback goes to a fixed set of public inbox relays.

**Release**
- Native v0.2.27 (protocol v95): the owner cuts it with the owner-held signing keys. The release now covers far more than it did when it was planned at protocol v64: the relay and privacy changes, Host Game on Fray, the real-multiplayer work, terrain v3, graphics settings and building variants. Until then `/download` serves v0.2.18. After publishing, republish the kind-30063 release event to the public relays.

**Decisions and sessions**
- Flipping new worlds to terrain generator v4 (dry caves) is an owner call; until then new worlds get v3.
- The default flip for single-player lending its world (D3, below) waits on a session with Axolittle.
- The owner session for shared steed packs (D2c-2).

---

## Next — design calls and foundations

Only items that block building further on top.

- **Planned dependency migrations** (held back from Dependabot on purpose): bincode 1 → 3 (changes the save and wire encoding, so it needs a migration with legacy-save tests) and wgpu/naga 29 → 30 (renderer API change; needs a visual check on real GPUs, native and WebGPU).
- **Route single-player through the server.** A host already lends its world to its embedded server (D1), so a hosting game no longer runs two simulations. Single-player still runs its own (D3: single-player lends too, with the default flipped after a session with Axolittle), and D4 moves the passes still owned by the client (clock and weather, mob locomotion, the death sweep) into the server tick. Both remain open. The `--no-lend` escape hatch and its second world copy go one release after D1 ships.
- **Server-owned joiner inventory, enforcement.** The server keeps a copy of each joiner's inventory, but the default Mirror mode still believes bounded claims. Strict enforcement is partly built and parked: it is selectable only internally and no operator flag is exposed.
- **`ServerPlayer` / `PlayerSlot` duplication.** The save path uses raw server-player fields while the client uses slots. They will drift; unify when saving becomes server-authoritative.
- **Comms ceiling raising.** The local policy file can only lower a player's chat ceiling. Raising it waits on a real capability boundary (a Charter comms clause or a Signet guardian attestation).
- **Anti-X-ray chunk obfuscation.** Built and unit-tested but not applied to the chunk push (`chunk_push::build_chunk_packets`): it only hides anything once the seed stops shipping to joiners, which regenerate natural ore from it. An owner decision. The architectural reward-layer defence is live; this part is not.
- **Take-your-worlds-to-native conflict policy.** Import currently keeps both when a name clashes; the final merge policy is an owner decision.
- **Sub-block detail for authored builds** (high-resolution statues and faces): needs a choice between a micro-model reference, sub-voxels or a detail-block palette.
- **Villager quests for joiners** (D2c-3). The common-progression foundation is merged; live quest delivery for joiners is not accepted yet.

---

## Later

Horizons, each one line. None is scheduled.

- **Farming tiers** — an eight-tier farming economy beyond the first tiers shipped.
- **Player-driven economies** — vendors, markets, auctions and bounties, extended on the settled non-custodial model (the engine holds a score, never a balance).
- **Rail logistics, remaining** — robbery reputation and bounty (needs a victim identity), wall-mounted cables, and phase 4 (Aether security). 45-degree ramps, cart tiers, depot-fed commercial vendors and the transit robbery record are already built.
- **Electricity phases 3–4** — quantitative energy economy, electric furnace and motor.
- **Aether (wireless signalling) element** — design written, not built.
- **Multi-world places, hub and portals** — one self-hosted address hosting several linked worlds, with a back-stack and per-world access rules.
- **Backup host** — a player the host names keeps a live copy of the world, so the session survives the host crashing or dropping off, not only a planned handover. Builds on world handover (now shipped); still a player's own machine, never one AxeNStax runs.
- **Creator gallery phases 3–4** — studio sidecar, and gated or consigned exhibits.
- **Voice and proximity audio** — game-transport-native, audio only; not built.
- **Guardian-facing controls** — permissions a parent or operator can set per child account; the operator-side allowlist exists, a guardian surface does not.
- **Companion screen** — a handheld as second screen and controller.
- **Networked spectator tiers** — full, limited and view-only.
- **Cloud save** — non-custodial, ciphertext-only, native and bring-your-own-key.
- **Self-hosting tooling** — backups, per-world configuration and a richer operator console, building on the dedicated server and Operator Console that exist today.
- **Cinematic camera and replay** — camera paths and recorded replays; partly built, not yet verified end to end.
- **Windows and macOS installers**, and an Android release.
- **Console and small-box hardware targets** (Raspberry Pi 5, mini PCs) and a GPU-tier auto-detect.
- **Texture-pack distribution** and WASM plugins for sandboxed server-side mods.
- **A Bitcoin-enabled server layer** — design only; see the red lines for the rules it would have to follow.
- **Historical-pivot and Genesis storylines** — the founding-myth arc (worthiness, not riches).
- **VR** — a reminder to explore headset play some day. Nothing planned, no date.

---

## Out of scope — red lines

The project does **not** and will **not**:

- Operate a public directory that lists player-run servers, worlds or groups. Discovery is LAN-local, direct-address, or an operator's own opt-in Nostr announce.
- Operate game servers, or relays that carry a group's game traffic, presence or in-game chat. Worlds are self-hosted; multiplayer runs directly between players.
- Collect children's data centrally, or collect age as raw data. The web taster stays anonymous and local; feedback is burner-key and end-to-end encrypted.
- Position or market itself as a social network, social media or a chat platform. It is a building game; playing with friends means building together.
- Offer chance-based real-money payouts. Proof of Play is educational proof-of-work. Any real-sats layer is off by default, parent-controlled, set per server, and driven by a deterministic work-meter, never a probabilistic threshold.
- Build a how-to for turning safety off. Sensitive features sit behind a real capability boundary, not a disclaimer.

The original Phases 6–7 (paid hosting fleet, matchmaking, platform services, celebrity-scale events) are **retired**: they would make AxeNStax the operator of the service, which these lines exist to prevent.
