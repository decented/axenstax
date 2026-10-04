# What's Coming

This page is a preview of what's **shipped recently**, what's **partway there**, and
what's **still to come** — so if you're wondering "is X in the game?", check here first.

The game is in **alpha**. Things change fast. This page reflects the state as of
**2026-09-07**; the authoritative, up-to-the-minute list is `docs/foundations/README.md`.

## Recently shipped (no longer "coming")

A big run of features landed across May–June 2026. These are **now live** — coverage in
the relevant Player Guide pages:

- **Electricity — the wired power tier** (Axe'n'Stax's take on redstone) — cables, levers,
  buttons, pressure plates, **logic gates** (AND/OR/NOT/XOR), generators, light-beam +
  motion sensors, and powered rails. See **[Electricity](electricity.md)**.
- **Explosives** — Black Powder, the **Blasting Keg** (fuse + plunger detonation), and the
  **Composter** (turns crop/seed waste into Saltpetre). Demolition only — no mining
  shortcuts. See **[Explosives](explosives.md)**.
- **Rails & carts** — the engine's first vehicle. Lay **Track**, ride a **Cart**, load and
  unload cargo at depot chests, and speed carts up with powered rail. (A dedicated guide
  page is on the docs to-do list.)
- **Building-detail blocks** — slabs (including **vertical** slabs), stairs, doors,
  trapdoors, fence gates, glass panes, iron bars, **signs** and **item frames** — all with
  proper shapes and collision. See **[Building Blocks](building-blocks.md)**.
- **Third-person camera (F5)** — opt-in third-person view with smart collision, avatar fade
  and feel settings.
- **Recipe book** — browse a 294-recipe catalogue and click a card to auto-fill the grid.
  See **[Crafting](crafting.md)**.
- **Graves + Keep Inventory** — die and your stuff waits in a grave; or toggle keep-inventory.
  See **[Cheats](cheats.md)** and **[Combat & Mobs](combat-and-mobs.md)**.
- **Minimap, waypoints & WorldEdit** — an in-world minimap, player waypoints, and built-in
  `/we` region tools (set/replace/walls/copy/paste/stack) plus Minecraft `.schem` import.
- **Art galleries & exhibits** — show off hi-res art in-world with `/exhibit` + a kiosk
  showcase mode. See **[Art Galleries](art-galleries.md)**.
- **Challenge Board (J)** and **Rig Studio (Y)** — around **45 authored Trials** that nudge
  you to try specific features across nearly every game system, and an in-game tool to author
  animated rigs that save with your world: pick a skeleton, pick a motion (Walk / Idle /
  Bounce), assign a block to each limb, Spawn. Limbs whose block has a baked micro-model are
  drawn as that little sculpture rather than a box. See **[Minigames](minigames.md)**.
- **Villages, Villagers & Raid Defence** — village procgen, 10 professions, gossip, Knights
  (village defenders), reputation, and **raids** (hostile waves attack a village treasury;
  defenders split the bounty). See **[Villages & Villagers](villages-and-villagers.md)** and
  **[Quests & Reputation](quests-and-reputation.md)**.
- **Wolves** — tame with bones (33% per click), sit/stand on command; tamed wolves drop
  nothing on death. See **[Wolves](wolves.md)**.
- **Dyes, fibre & dyed décor** — the **16-colour** dye paintbox, farmable flowers + cotton +
  hemp, Rope/Lead/Cloth/Canvas, and dyed décor (wallpaper, bunting, paper lanterns, kites,
  banners, sails, tents). See **[Dyes, Fibre & Magnesium](dyes-fibre-and-magnesium.md)**.
- **Cyanotype blueprints** — capture a build (or wall art) onto Blueprint Paper, develop it
  in the sun, then rebuild it or hang it as décor. See **[Build Schematics](build-schematics.md)**.
- **Leads & fence posts** — craft a Lead to walk animals around and tie them to fence posts.
- **Inventory Explorer (B)**, **stone tools + cobblestone**, **stone variants + Copper Ore**,
  **6 wood species**, **8 biomes with blending**, the **Furnace**, the **Vendor Block**,
  **Plaque tipping**, **Bulk Vendor** mode, **Chainmail** (Marauder drop), and the
  **armour data layer** all landed earlier in the run.

A Minecraft-parity **gap-closure wave** landed 2026-06-21:

- **Bonemeal grows crops** — right-click Bonemeal (1 Bone → 3 Bonemeal) on a growing crop to
  jump it 1–2 stages. See **[Farming](farming.md)**.
- **Animals flee when struck** — hit a cow/sheep/pig/horse/rabbit/goat and it bolts.
- **Tamed wolves follow you** — your wolf now trots after you (sit to make it stay).
- **Beds** — sleep through the night and set your respawn point (right-click a bed at night).
- **Animal breeding** — feed two adults their food (cow/sheep/goat = wheat, pig/rabbit =
  carrot, chicken = seeds) and they make a baby that grows up.
- **Fishing** — right-click a Fishing Rod at water, wait for a bite, right-click to reel in a
  catch; cook Raw Fish on a campfire.
- **Hoppers** — craft a hopper (5 iron + a chest) and place it between two chests to move
  items down automatically.

More landed since then:

- **Animal AI flavour, all four pieces** — the rabbit's bouncy hop, the goat's occasional
  charge, the bee's sting-back, and **tamed wolves fighting alongside you in combat** are
  all live now.
- **Armour equipping** — a full drag-to-equip UI plus a live HUD armour-points readout. See
  **[Armour](armour.md)**.
- **NPC Builder commissions** — hire a Builder at the Drafting Table: pick a Plan, see the
  materials + fee preview, confirm, and the Builder pathfinds over and constructs it.
- **Sticky pistons** — the sticky variant pulls its block back on retract, same as the plain
  piston pushes forward. See the [Electricity guide](electricity.md).
- **Wind, Copper & Electricity wave** — **Copper Ore** now generates underground (a mid-depth
  stone band), so the whole electricity chain — dig, smelt, wire it up — is a normal Survival
  playthrough for the first time. The **Windmill** turns in real, weather-driven wind (build it
  high for a reliable breeze). A batch of electricity bugs got fixed along the way: a Battery no
  longer tops itself up off its own wire forever, and a friend's own switches (Lever, Button,
  Hand Crank, Plunger Detonator, Mirror) now work on your hosted world. Seven new Trials cover
  it — see **[Minigames](minigames.md)**. See **[Electricity](electricity.md)**.

And a lot more since then (August–September 2026):

- **Native mailbox** — an alpha-tester feature, off by default (Settings in the lobby or pause menu, then tap the version line 7 times to switch it on). Send `/bug` / `/idea` from the desktop app (each report is sealed
  with a fresh one-time key, no account or name attached), and see what happened to them
  with `/mailbox`: Sent, Received, Fixed in vX, or Won't fix. We never message players —
  the status comes from a public list of scrambled ticket numbers only your own game can
  recognise. Reports are stamped with the game's build version. (The browser version has no
  feedback channel — `/bug`, `/idea` and `/mailbox` are desktop-app only.)
- **Skin painter controls** — **Tab** opens a full paint panel (colour wheel, a `#rrggbb`
  box, all 16 dyes, your last 8 colours), **R** moves your avatar's limbs apart so you can
  reach every face, and there's a Classic/Slim arms toggle. See **[Skins & the Wardrobe](cosmetics.md)**.
- **Skindex-parity painter** — six tools (Brush, Fill, Eraser, Lighten, Darken, Noise), a
  Shift-click straight-line tool, a pixel grid with a hover box, and Slim ("Alex") arms
  alongside Classic. See **[Skins & the Wardrobe](cosmetics.md)**.
- **Weather sync (multiplayer)** — the host now owns one rain/storm window and every
  player sees the same weather, instead of each computer rolling its own.
- **Native version badge + in-place update** — the desktop app tells you when a newer
  build is out (checked over the web and, as a second source, over Nostr) and can install
  it itself, in place, over an AppImage.
- **The Workshop's eyedropper (G)** now reads transparency correctly, and the Bellows
  shows an aim highlight while you're lining up a blow-up. See **[The Workshop](workshop.md)**.
- **Full-fidelity item wire** — a tool or piece of armour someone drops now keeps its exact
  durability and enchant-free stats over the network, not a fresh copy.
- **Guided build-along** — for a saved Plan, `/buildguide` (or the in-game prompt) projects
  it as a ghost you build into, block by block, layer by layer, or all at once.
- **Water Wheel** — the first free-running power generator; place it in a flowing stream
  (not a pond) and it needs no fuel. See **[Electricity](electricity.md)**.
- **Rig Studio** gained squash-and-stretch micro-model shells and a **Bounce** motion, on
  top of Walk/Idle. See **[Controls](controls.md)**.
- **`.axeprofile`** — "Export everything" (web) bundles every browser world plus your
  Trials records into one file; "Import a web profile…" (desktop) brings them in, keeping
  both copies if a name collides. See **[Cloud Saves & the Stash Column](cloud-and-stash.md)**.
- **Crosshair hover labels** on every power device (fuel gauge, charge, on/off) — aim at
  one and it tells you its state without opening anything.
- **Mipmaps + anisotropic filtering**, an opt-in Graphics dial that smooths distant
  textures. See **[Tips & Tricks](tips-and-tricks.md)**.
- **World chat** — type a line without a leading `/` in the chat overlay and it's a real
  message to the other players in the world, not a local echo. **Desktop app only**, and it
  needs you to be signed in — no verified identity, no chat. Who hears you depends on how
  you and they have recognised each other (family/friends can talk both ways; someone
  you've merely recognised can be heard but can't be spoken to). There's no parental
  control over it yet — see **[Chat & Commands](chat-and-commands.md)**.
- **Online Play by Contact** — host a world at home and a friend in another house can join
  it by your **npub** or a bearer invite link, no server of ours involved. Public Nostr
  relays carry only the encrypted setup handshake; the game itself connects the two
  computers directly. **Brand new** — not yet tried by the team across two real houses.
  See **[Play with a friend online](play-with-a-friend-online.md)**.

## Partially shipped — finish-the-loop work pending

### Cinematic Camera (the Director)

The built-in freecam + film studio is **live on PC** — fly a detached camera with six
modes (free-fly, path/dolly, tripod, follow, look-at, POV), hide the HUD for a clean frame,
and **record** the session to an `.axereplay` file (**F4**). See **[Cinematic Camera](cinematic-camera.md)**.
The remaining half is **in-game playback** — re-flying the Director *through* a recording
with slow-motion and timeline scrubbing — plus filming **other players** in multiplayer.
For now, footage is captured by screen-recording the clean Director view. *(Native desktop
build only; not in the browser/PWA.)*

### Farming Tier 1.5 — processed food economy

Tier 1 farming is fully live (wheat, carrots, potatoes, corn, sugar beet, beetroot, berries,
pumpkin, papyrus, cotton, hemp, dye flowers — till, plant, grow, harvest). The bigger
**processed** economy — Mill, Oven and Aging Rack workstations; flour/dough/cheese/butter and
baked goods; milk-from-cows and eggs-from-chickens — has its data layer in and live
workstations coming. See **[Farming](farming.md)**.

### Electricity Phase 3

Phases 1–2 (the wired grid + sensors) shipped, and so have both free-running
generators — the **Water Wheel** (needs a *stream*, not a pond) and the
**Windmill** (needs open sky and, ideally, some altitude). See
**[Electricity](electricity.md)**. Analog signal **strength** and an energy
economy are what's left of Phase 3.

## Not yet shipped (specced + ready to build)

- **Farming Tiers 2–8** — orchards, ranching, beekeeping, aquaculture, industry, biotech.
  A long roadmap; each tier derives its own spec after the previous one is playtested.
- **Village procgen from the Plan Registry** — the registry framework is live; it needs a
  batch of authored house plans to take over from the single hardcoded house shape.
- **The wider player economies** — markets, services, combat, PvP, land, knowledge and
  spectator economies beyond farming.

## Blocked (waiting on the owner or upstream)

- **Live multiplayer auth test** — joining a remote server with your Nostr identity is built
  on both sides (the handshake signs your identity on join); it needs the owner's two-machine
  LAN + bunker test to flip from "built" to "verified".
- **Online Play by Contact, live across two houses** — the code path (invite exchange over
  Nostr, router punch-through, then a direct connection) is built and unit-tested, but it
  hasn't yet been run for real between two separate homes. See
  **[Play with a friend online](play-with-a-friend-online.md)**.
- **Single-player → server unification** — single-player still runs a slightly different
  simulation path from multiplayer. Merging them is a high-risk refactor that wants Axolittle
  present for phase-by-phase regression.
- **Real-money layer** — not built, and not promised. The Proof-of-Play hash, Satori veins and
  Genesis Block are live as educational proof-of-work and in-game scoring; sats numbers in
  trades are an internal score only. Any future layer would be off by default, per server and
  parent-controlled.

## Want to track progress?

The full status table lives in [`docs/foundations/README.md`](../foundations/README.md).
Each spec has a status field (READY TO BUILD / PARTIALLY DELIVERED / DELIVERED / BLOCKED) and
a one-line summary.

## Things that AREN'T in the game (and may not be)

To prevent confusion:

- **Repeaters / comparators (timing & analog signals).** Not in yet — Electricity is on/off
  only for now. (You *do* have wires, logic gates, switches, powered rails, hoppers that move
  items, and **pistons** that move blocks — see the [Electricity guide](electricity.md).)
- **Smooth piston push animation.** Both the plain **piston** (shoves blocks when powered) and
  the **sticky piston** (pulls its block back on retract) are in and tested; only the polish
  of a smooth push/pull animation is still pending.
- **Enchantments / magic.** Out of scope for alpha.
- **Brewing / potions.** Out of scope for alpha.
- **The Nether / End dimensions, and boss mobs (Ender Dragon / Wither).** Not specced — the
  game is single-world for alpha. The Aether dimension is discussed in research but not
  committed.
- **Trading between villagers (Minecraft emerald economy).** Deliberately skipped — trade
  happens between PLAYERS via the Vendor Block (or via Builder commissions).
- **Boats, crossbows, shields/off-hand, music discs, marriage/family trees, VR.** Not in /
  not committed.
- **Llama.** Deferred. (Wolves, Cats, Foxes and Parrots are all in — see [Wolves](wolves.md) and [Combat & Mobs](combat-and-mobs.md).)
- **Coloured wool / dyeing live sheep.** The dyes themselves **shipped** — but dyeing a live
  sheep or making coloured wool isn't in yet.
- **Parental control over world chat.** World chat shipped (see above), but a guardian
  dialling it down or off isn't wired yet — the plumbing it'll eventually use doesn't exist
  upstream yet either.
- **Authoring your own Experience.** You can adopt a skin or play an Experience someone else
  has published (see **[Cloud Saves & the Stash Column](cloud-and-stash.md)**), but there's
  no in-game tool yet for building a new Experience of your own to publish.
- **Others seeing your worn skin in multiplayer.** Your skin shows on your own screen; other
  players seeing it too is on the way.

If you want a feature, tell Staxolottle. The roadmap is community-shaped.
