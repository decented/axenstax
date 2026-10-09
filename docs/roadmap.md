# Axe'n'Stax Roadmap

*As of 2026-10-04 — source v0.2.27, network protocol v64. The repository is public and the sites deploy from `main`.*
This file is **the** roadmap: where the project is, what is waiting, and what comes later.
For "what do we build next", see `docs/foundations/README.md` (the build queue); feature requests and ideas enter through the internal feature backlog.

---

## Where we are

Axe'n'Stax is an open-source, self-hostable voxel sandbox with its own engine, client and server, written in Rust. The build queue is empty: everything that was ready to build has been built. What remains is work only the owner can do (cutting the native release, and live tests with real devices and people), plus a short list of design decisions that block building further on top of the current foundations.

- **Version:** v0.2.27 in source, network protocol v64. The published desktop installer is still v0.2.18 until the release below is cut.
- **Web (`play.axenstax.com`)** is an anonymous, local-only sandbox: a *taster*. No login, no multiplayer, no cloud save and no feedback channel on web. Worlds live in the browser and can be exported to the desktop app.
- **Native (Linux AppImage)** is the full game: Signet sign-in, LAN play, online play by contact, the dedicated server, and the feedback mailbox. Windows and macOS are not published as installers yet (build from source).
- **Android** is ported onto the current main (v0.2.27 APK builds in CI, `android` input off by default), but nothing is published: release signing is owner-side, and updates will come through zap.store, not an in-app updater.
- **Gamepad support** is built and working, but is parked as a launch priority.
- **Posture:** self-hosted, sovereign, player-owned. Worlds, saves and identity belong to the player; AxeNStax ships software, not a service. See the red lines at the end.

---

## Shipped

### Engine and rendering
- Custom Rust engine on wgpu: native desktop and WebAssembly/WebGPU (Chromium) from one codebase.
- Greedy meshing, texture-array atlas, day/night cycle, weather, water and lava flow, lighting, particles, snow layers, per-species mob tint, see-through glass and leaves, optional mipmaps.
- Resolution-agnostic texture packs: disk packs with a manifest, a picker, and hot-swap without a restart.
- Headless test harness that boots the full client without a display; a single regression gate (`check.sh`) covering lint, tests, the WASM build and a bundle-size limit.

### World and gameplay
- Survival and creative modes, difficulty selector, and a one-way World Integrity Ledger (pure survival / ever creative / cheats used).
- Tool progression, smelting, crafting (2x2 and 3x3) with a recipe book, repair, armour, hunger and health, death drops and respawn.
- World generation with biomes, caves, ravines, mineshafts, villages, brigand hideouts and coal, iron, diamond, copper, magnesium and nitre ores (no tin, gold or amethyst source yet, so Bronze is unreachable in survival).
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
- Schematics and blueprints with a build guide, a Drafting Stamp, and world-edit tools.
- The Workshop: reskin and reshape blocks and mobs in game (paint, carve, symmetry, per-block wardrobe) and a Rig Studio for animated assets.
- Skin editor with Minecraft skin import and export, classic and slim arms, and painter tools.
- Exhibits (images on walls and standing frames) and a kiosk mode.
- Rails with auto-connecting bends, 45-degree ascending and descending ramps, and flat floor cables; Rail Freight phase 1, plus parts of phases 2 and 3: cart hull tiers (wood, iron, diamond), bulk vendors restocking from an adjacent depot chest, and a transit-only robbery record (`hostile_acts`).
- Cameras: opt-in third-person view (F5).
- Take-your-worlds-to-native: the web lobby exports one profile file, and the desktop app imports it (name clashes keep both).

### Multiplayer and identity
- LAN co-op over QUIC, split-screen on one machine, and a headless dedicated server (Docker image and an Operator Console with allowlist, blocklist and require-sign-in). Known limits: the dedicated server's `GameServer::tick` does not run pistons, hoppers, kegs, dispensers or crops (so farms and machines do not run there); a joiner's world edits are applied without a possession check; no `StateUpdate` size bound; and a late joiner gets no earlier world edits or chunks (see Next).
- Online play by contact (native): join a friend's home-hosted world by their Signet persona npub or a bearer invite. Nostr relays carry connection setup only; the connection is then direct. Strangers get silence. Relays never carry game traffic.
- Server authority over remote players (movement, reach and block edits are validated; inventory is not server-authoritative); join handshake with Signet-signed identity and protocol framing (v64).
- World chat (native), tiered by contact relationship, with a ceiling that can only be tightened.
- Signet sign-in by QR (mySignet pairing) and Signet contacts sync: Kin, Kith and blocks feed the address book, and a block removes a player from a running host.
- Optional, self-published Nostr announce for an operator's own server. Nothing is ever announced automatically.
- Death-drop items and late-joiner entity backfill over the wire.

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
- Death drops, late-joiner backfill and tool pickup, walked over by a second player.
- Signet contacts sync on a phone: pairing, Kin/Kith appearing, a block kicking a connected player, disconnect. This now also tests the relay change: pairing runs over the first of "Your relays" (a public relay by default), not the project's own relay.
- Native QR sign-in on a phone with mySignet. This now also tests the relay change: the QR advertises the first three of "Your relays" (public by default), and editing the list from the sign-in screen rebuilds the QR.
- A `/bug` report reaching the project's public inbox relays and being read back by the feedback reader, and the release event republished to the public relays so the in-app updater (which now reads the player's relays) finds it.
- Playtest of the unverified trials, and the electricity switch test across two machines.
- First-launch data-directory migration, the updater, and the v64 framing on real hardware.

**Fun test**
- Play-testing with voluntary children who are not family. Family play has been a bug-hunt, not evidence that the game is fun, so fun is unvalidated.

**Public launch — done 2026-10-03/04**
- The repository is public as a fresh single-commit snapshot (old history archived privately), with fork-PR approval, secret scanning and push protection, private vulnerability reporting and a protected `main`.
- The privacy page is signed off by the owner (2026-10-04): controller published as *decented*, bug reports kept until dealt with and at most 6 months.
- The project's own relay is no longer a default anywhere in the native app or its tools (pending the phone tests above). Sign-in, contacts pairing, server discovery and the updater use the player's one editable "Your relays" list; feedback goes to a fixed set of public inbox relays.

**Release**
- Native v0.2.27 (protocol v64): the owner cuts it with the owner-held signing keys. Until then `/download` serves v0.2.18, which still predates the relay and privacy changes. After publishing, republish the kind-30063 release event to the public relays.

---

## Next — design calls and foundations

Only items that block building further on top.

- **Planned dependency migrations** (held back from Dependabot on purpose): bincode 1 → 3 (changes the save and wire encoding, so it needs a migration with legacy-save tests) and wgpu/naga 29 → 30 (renderer API change; needs a visual check on real GPUs, native and WebGPU).
- **Late-joiner chunk streaming.** The server backfills entity spawns to a late joiner, but the client renders only dropped items from them (mob and cart spawns are ignored, `remote_entities.rs`); world edits and chunks for a late joiner need a call between edit-log replay and chunk push. Multiplayer chunk compression rides on the same decision.
- **Route single-player through a local `HostedServer`.** Single-player still runs its own mob, spawning and falling-block simulation alongside the server's (the "dual sim"). Ending it needs a playtest that validates entity behaviour with the client-side sim removed. Kill attribution, bounties and vows stay client-side until then.
- **`ServerPlayer` / `PlayerSlot` duplication.** The save path uses raw server-player fields while the client uses slots. They will drift; unify when saving becomes server-authoritative.
- **Local-player position trust.** Remote players are server-simulated with a speed cap; the host's own local player is still position-trusted. Resolved by the single-player routing above.
- **Client-asserted join name.** The signed identity is authoritative and the typed name is a display fallback only; the remaining work is the live two-machine verification above.
- **Comms ceiling raising.** The local policy file can only lower a player's chat ceiling. Raising it waits on a real capability boundary (a Charter comms clause or a Signet guardian attestation).
- **Anti-X-ray chunk obfuscation.** Built and unit-tested but not yet in the live chunk-stream send path; it lands with real remote chunk streaming. The architectural reward-layer defence is live; this part is not.
- **Take-your-worlds-to-native conflict policy.** Import currently keeps both when a name clashes; the final merge policy is an owner decision.
- **Sub-block detail for authored builds** (high-resolution statues and faces): needs a choice between a micro-model reference, sub-voxels or a detail-block palette.
- **World handover.** A host who needs to leave can hand the running world to another player in the session ("Hand this world to …"): the world and everyone's saved things move to that player's machine, and everyone reconnects there. Queued in the multiplayer work after per-player joiner saves; the original host keeps their own copy.

---

## Later

Horizons, each one line. None is scheduled.

- **Farming tiers** — an eight-tier farming economy beyond the first tiers shipped.
- **Player-driven economies** — vendors, markets, auctions and bounties, extended on the settled non-custodial model (the engine holds a score, never a balance).
- **Rail logistics, remaining** — robbery reputation and bounty (needs a victim identity), wall-mounted cables, and phase 4 (Aether security). 45-degree ramps, cart tiers, depot-fed commercial vendors and the transit robbery record are already built.
- **Electricity phases 3–4** — quantitative energy economy, electric furnace and motor.
- **Aether (wireless signalling) element** — design written, not built.
- **Multi-world places, hub and portals** — one self-hosted address hosting several linked worlds, with a back-stack and per-world access rules.
- **Backup host** — a player the host names keeps a live copy of the world, so the session survives the host crashing or dropping off, not only a planned handover. Follows world handover; still a player's own machine, never one AxeNStax runs.
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
