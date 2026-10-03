# Multi-world Places, hub/menu navigation & portals — design (long-run)

**Date:** 2026-06-22
**Status:** DESIGN (no code) — north-star spec; phased so Phase 1 ships with little/no engine risk
**Owner ask:** A dedicated server should be able to host **multiple worlds**, reachable from
**one address**. A visitor entering that address gets either a **menu** of worlds or is dropped
into a **lobby world** — host's choice, and both must be supported. Navigation must **remember
where you came from** so "leave" returns you there, and **each world configures its own exit**
(back / to the lobby / to the menu / elsewhere). Worlds have **per-world reachability** (some
only via the lobby, some only via a direct link). And the endgame is **full portals**: walk from
any world to any other world (or back to the lobby), not just hub-and-spoke. All configured in the
operator console ("home admin panel").

Supersedes the single-world assumption in `2026-06-16-dedicated-docker-server-design.md` and
extends `2026-06-21-server-setup-wizard-design.md`. Aligns with Spec 07 §8 (Portals / world
lifecycle) — this is the dedicated-server-scale path toward it.

---

## 1. North star (the endgame)

**A "Place" is a navigable graph of worlds — the spatial web.** One address (a Place), many
worlds (pages), portals (links) connecting them, a back-stack (history), a menu/lobby
(site-map / home), and per-world access rules (public/unlisted/private). The long-run endgame:

- Enter a Place by its address → see its **menu**, or land in its **lobby**, or follow a **direct
  link** straight to one world.
- Inside, **portals** carry you world→world arbitrarily (A→B→C), authored by the host — not just
  hub-and-spoke through one lobby.
- Every hop remembers the trail; **exits are configurable per world** (go back, go to the lobby,
  go to the menu, go to a specific world, or leave the Place).
- Eventually, portals cross **Places** too (a world on one box → a world on another), via a world
  directory — that's the Spec 07 fleet horizon.

Design principle that keeps this coherent with existing decisions: **within a single world,
travel stays physical** (walk / rail / cart — see the rail-replaces-waystones decision); **between
worlds, travel is a portal** (a connection + handoff). Portals are *not* in-world teleporters;
they are links between separate world instances.

### Build elsewhere, publish from the game (content vs arrangement)

The spine that stops the interfaces multiplying: **you never build on the server.** You build worlds
in the **normal AxeNStax client** (entrance → lobby → your worlds), where creative, the block
palette, and saving already work — and your worlds live in your **Stash** (your private cloud
library, syncing across your devices). The dedicated server only **serves** worlds you publish to it.

Responsibilities split cleanly:
- **Content** (the actual builds) = the normal game + your Stash.
- **Arrangement** (which worlds are live, the lobby/menu, portals, exits, access) = the console.

**Initial publish from the console; updates from the game** (owner refinement, 2026-06-22) — two
steps:
1. **Create / register a served world — in the console** (the existing setup wizard, generalised to
   "add a world"). This is where you decide a world joins *this* server and how: its **kind**
   (gallery etc.), **biome/terrain**, **access**. The "initial publish." It's *arrangement*, so it
   lives with the console — and it needs **no Stash-signer**: it provisions a world + gives you an
   editable copy in your game; it doesn't pull encrypted content.
2. **Edit + re-publish — from the game.** Thereafter the world is yours to edit in the normal client,
   and the build→publish loop is tight: **Publish** lives on the world in the lobby (and/or the
   in-world pause menu) — never a console trip. Save to Stash = just save *this version* (private);
   **Publish** = push it live when ready.

(Your operated servers appear as publish targets because your signed-in npub matches the server's
operator identity — the "My Servers" list in its operator role.)

**Website-style draft → publish — this replaces the gallery build/open toggle entirely.** You edit
your *own copy* in the game; visitors keep seeing the *last published* version until you push an
update. Editing never disturbs live visitors, and a served world is read-only to them by nature — so
there's no "build mode locks everyone out" dance. Re-publishing overwrites the live copy (the server
keeps the previous version as a backup, like the wizard's world archiving). Publishing is
**operator-only** (npub must match the server).

**Stash (save) and Publish (release) are independent — you never toggle Stash off.** Saving to Stash
is your private cloud backup (per world, frequent/automatic, syncs across your devices) and **does
not touch any server**. **Publish** is the *only* action that pushes to a server, and it pushes a
snapshot of your *current* world — so you can stash WIP freely (back it up, sync to another device)
without anything going live, and the server keeps serving the last published version until you
deliberately publish again. It is save-draft vs publish / autosave vs deploy. To keep it clear, each
world shows a **publish-state badge**: *not published* / *published to <server> · up to date* /
*published · you have unpublished changes* — so you always see what's live vs what you're editing,
and "Publish" simply means "make the version I'm looking at the live one." (Optionally, when Stash
keeps version history, Publish may push a *chosen* saved version; the default stays "publish
current.")

**A world carries its own publish record — this is how the lobby knows.** Each world stores in its
metadata (which **travels with it in Stash**, so every machine agrees): whether it's been published,
**which server** (operator npub + address), the **kind-of-place** chosen, and the **content version
last published**. The lobby reads this to draw the badge; *unpublished changes* = current content
differs from the last-published version (a content hash/version stamped at publish, or a dirty flag
since last publish). Two **independent** properties live on a world:
- **Stash state** — saved/synced. **Universal: every world is stashed for safety**, whether or not
  it's ever served — so "back it up / swap machines without publishing" just works.
- **Serve state** — opt-in, per world: *personal (not served)* → *set up to serve `<server>` as a
  `<kind>`* → *published · up to date* ↔ *published · unpublished changes*, with *unpublish* back to
  personal.

So it is **not a separate kind of world** — any world can *become* servable. You build and stash
freely; **"Set up to serve / Publish…"** is the moment a world gains a target + kind-of-place; until
then it's a plain personal world with no publish clutter. (Optional convenience: ticking "I'm making
this to serve" at *creation* just pre-sets the kind + primes Publish — it doesn't change that the
world is stashed like any other, or force a publish.)

**Mechanism (security):** publishing pushes the world **straight from the game to the server,
authenticated as the operator** — the world is already loaded in the game and the game already holds
your signer, so it's the least-plumbing path and the **server never holds your keys**.

**Bonus — it sidesteps a current blocker:** because publishing happens in the *game* (where Stash +
the signer already work), the **console never needs the Stash-signer wiring** that's currently blocked
(an upstream Signet app change). The console only ever *arranges* worlds that are already published.

**Boundary:** this is the model for **curated / single-author worlds** (galleries, adventure maps,
anything you build then show) — the bulk of the use cases. **Live collaborative building** *on* the
server (friends editing one world in real time) is a separate capability needing a writable server
(the web-edit-propagation work); it can coexist later.

### What a "gallery" is — a capability, not a restrictive type (owner Q, 2026-06-22)

A **gallery is not a separate, limited kind of world.** It's a **capability you switch on for any
world**, and it does exactly two things:
1. **Unlocks image/art assets** (the "My Art" library + exhibit placement) — these are
   **gallery-only** (a normal world can't place arbitrary images; this also *contains the
   content-safety surface* to gallery-labelled, published worlds).
2. **Makes visitors read-only** (the look-don't-touch / kiosk experience).

Everything else is **a normal world underneath** — any **base mode** (creative, to build it) and any
**biome/terrain**. So you can **spin up a waterfront / forest / flat world and mark it a gallery**
rather than hand-build all the scenery; the gallery flag doesn't care about terrain.

**What stops it being "just a normal world" is intrinsic, not an artificial limit:** a gallery's
visitors are **read-only**, so you'd never label a *collaborative-build* world a gallery (nobody could
build together). That single, desirable property *is* the whole distinction — no extra restrictions
needed, and a gallery can be as plain or as curated as the creator likes (a feature, not a bug).

So in the wizard/console, "kind of place" is really *base mode + biome + an optional **Gallery**
capability*, not four rigid types. **This RESOLVES the earlier art-access fork:** art lives in
**gallery-labelled worlds**, any biome.

---

## 2. Core concepts (vocabulary)

| Term | Meaning |
|---|---|
| **Place** | One dedicated server (one Docker deployment), reachable at one address. Hosts ≥1 world. |
| **World** | One voxel world instance. Has id, title, type (gallery/creative/survival/adventure), spawn(s). Its content is a **published snapshot** (built in the normal client, pushed from the game), not edited on the server. |
| **Front door** | What a *fresh* visitor to the Place lands on: the **menu** or the **lobby** world. |
| **Menu** | The launcher-level world picker, rendered by the client from the Place's manifest. |
| **Lobby** | A designated world that doubles as an in-world hub (portals/doors to other worlds). |
| **Direct link** | An address that points at one specific world, bypassing the front door. |
| **Portal** | A placed in-world connection from a point in world A to an anchor in world B (or a return). |
| **Reachability** | Per world: which routes may reach it — menu / lobby / direct / portal. |
| **Back-stack** | The trail of how the player got to the current world; "leave/back" pops it. |
| **Exit policy** | Per world: what "leave" does (back / lobby / menu / specific world / out of Place). |
| **Handoff** | The mechanism that moves a player between worlds (reconnect + a token carrying identity, return target, and a route ticket). |
| **External link** | A navigation target that points *off* the Place to a normal website (e.g. a shop). Web-native; native opens the system browser or shows a QR. |

---

## 3. Addressing & discovery

- A Place publishes a **manifest** at a well-known path (Caddy-served, console-managed):
  `https://<place>/worlds.json`.
- A client given a Place address **fetches the manifest** and acts on it:
  - manifest present, `front_door: "menu"` → render the **world picker**.
  - manifest present, `front_door: "lobby"` → join the **lobby** world directly.
  - manifest absent → **join the single world** (today's behaviour; fully backward-compatible).
- A **direct link** carries a world id (`…/worlds.json#studio` or a world-specific connect URL) →
  the client skips the front door and connects straight to that world (subject to its reachability).
- Works for **web and native** identically (native's Join box fetches the same manifest), which is
  why discovery lives in a manifest, not a web-only hub page.

Addressing of the world socket itself (from the prior discussion):
- **Web** must ride one HTTPS origin (cert + secure-context), so worlds are distinguished by
  **path**: `wss://<place>/ws/<world>` (Caddy routes each path to that world's engine).
- **Native** may use the path too, or connect to a **port per world** directly
  (`ws://<place>:6767` …): the engine socket needs no `/ws` label.

---

## 4. Entry points: menu + lobby + direct (all three, coexisting)

The menu and the lobby are **layers, not alternatives**:
- The **menu** is the top of the trail (the picker).
- The **lobby** is a world that is *also* a menu entry *and* an in-world hub.
- `front_door` only chooses what a bare visitor sees first; from the menu you can enter the lobby,
  and from the lobby you can (if configured) get back to the menu or out.

A direct link drops the visitor into one world with an **empty trail above it** (they came from
"outside") — see exits, below.

---

## 5. Navigation: the back-stack + per-world exit policy

The client maintains a **back-stack** (breadcrumb) of the worlds it has passed through this
session, starting from the entry point (menu / lobby / direct-outside). Each portal/door hop
**pushes** the current world; "leave/back" **pops**.

Each world then declares an **exit policy** that decides what "leave" actually does — this is the
"where should it exit to?" the owner asked for:

| `exit` value | Behaviour on leave |
|---|---|
| `back` (default) | Pop the back-stack — return wherever you came from. |
| `lobby` | Always return to the Place's lobby world (ignore the trail). |
| `menu` | Always return to the world picker. |
| `world:<id>` | Always go to a specific world (a fixed "exit door"). |
| `out` | Leave the Place entirely (e.g. a kiosk dead-end / the gallery exit screen). |
| `url:<href>` | Leave to an external **website** (e.g. the gallery's shop). Web navigates; native opens the system browser / shows a QR. See §6 *External targets*. |

Worked examples:
- Menu → Lobby → Gallery, all `exit: back`: leave Gallery → Lobby → Menu → out.
- A `maze` world with `exit: lobby`: however you got in, leaving always returns you to the lobby.
- Direct link → Gallery (`exit: back`): trail is empty above it → leave = out of the Place.

Portal vs back-stack: taking a portal A→B **pushes A** (so B's `back` returns to A) **unless** B's
exit policy is a fixed target, which overrides. Cap/dedupe the stack to avoid unbounded loops.

This is, deliberately, **browser history for worlds**: links push, back pops, and a page can
declare "always send back to home".

---

## 6. Portals (the endgame)

A **portal** is a host-authored connection placed in a world:

```
portal := { from_world, from_anchor, to_world, to_anchor|"spawn", policy }
```

- **Arbitrary topology.** Portals connect any world to any world — A→B→C→A — not only through the
  lobby. The lobby is simply a richly-connected node. "Enter through the lobby, then hop world to
  world" is just one topology among many.
- **Authoring.** Portals are placed in **build mode** like exhibits: an in-world *portal block* (or
  `/portal` command) whose destination is set by the operator; mirrored/managed in the console.
- **Traversal.** Walking into / activating a portal triggers a **handoff** to the destination
  world + anchor. Near-term that's a guided reconnect (brief load); long-term it's seamless
  (preloaded destination, no visible drop).
- **Return.** The handoff pushes the source onto the back-stack and carries a **return target**, so
  the destination's `exit: back` knows where home is even across the reconnect.
- **Boundary (keeps rail/waystone decision intact):** portals move you **between worlds**;
  **inside** a world you still travel physically (walk/rail/cart). No in-world teleport creep.

### Aether — why portals aren't "magic" (summary; full design → the Aether spec)

Portals run on **Aether**, the *wireless* element ("connection without a wire"; Electricity = wired).
They stay "unexplained, not supernatural" and don't undo rail-over-waystones:
- **Travel rule:** physical travel *within* a world (rail/cart/walk); **Aether bridges what physical
  space can't connect** (between worlds + pocket interiors) — never a shortcut for a walkable route.
- **Engineered = gates:** built, powered by an Aether **core**, **attuned** to one partner (= the
  handoff token in code). Costs: one-time attunement + per-trip charge + optional upkeep + scope
  scaling (pocket cheap → cross-Place dear). **Failure is safe** (no charge ⇒ won't open).
- **Wild = astral** (an Aether-fungus; spirit travels, body stays — scouting, not logistics; risky).
  Loop: forage → astral-scout → attune → build a gate (explorer's tool → settler's tool).

**The full Aether design — the cost model, the engineered/living wireless faces + the humane rule,
astral travel, and the mushroom-foraging danger + child-safety framing — now lives in its own spec:
`2026-06-22-aether-element-design.md`. Make Aether changes there, not here.**

### External targets — links off the Place (e.g. gallery → shop)

A navigation target (an `exit` or a portal/door) can point at an **external website** instead of
another world: `url:<href>`. The motivating case: a **gallery's exit takes the visitor to the
artist's shop** to buy the art. In the spatial-web metaphor this is just a hyperlink that leaves
the Place for the wider web.

It's a **terminal** target — it leaves the experience, so it doesn't push onto the back-stack.

**Per-platform behaviour (the owner's "might not work for native"):**
- **Web (the primary case):** open the URL — preferably in a **new tab** (the gallery stays open
  behind it) or, for a true "exit" screen, replace the page. Natural and frictionless.
- **Native:** *not impossible* — open the OS **default browser** (the engine is native Rust; a
  one-liner via a `webbrowser`/`open` helper). Where that's unwanted or unavailable, show an
  **in-game panel with the link + a QR code** (scan with a phone — nice for TV/handheld), and if
  even that's off, fall back to the world's normal exit. So native degrades gracefully rather than
  breaking.

**Safety — this is a trust + child-safety surface, not just a feature.** A Place sending visitors
to arbitrary external sites is a phishing/scam vector, and the audience skews young:
- **Always show a "you're leaving" confirmation** with the **destination domain** before navigating
  — never silently redirect off the Place.
- The manifest **declares** outbound links (`label` + `href`), so they're visible/auditable rather
  than hidden in world data.
- **Tie to the platform's age-safety posture** (COPPA / UK OSA — see the compliance research): outbound
  links, *especially shops / anything payment-adjacent*, should be **gateable or disabled for child
  accounts**, and a shop link is a commerce surface the operator is responsible for. This is a
  policy hook, not just UX.

**Creator-economy hook (future):** the same `url:` target generalises to **per-exhibit "buy this"
links** — each artwork → its product page — turning a gallery into a storefront. Out of scope to
build now; the target type is designed so it slots in later.

Manifest shape for an external target (used by an `exit` or a door):
`{ "type": "url", "href": "https://ada.example/shop", "label": "Visit the shop", "confirm": true, "open": "newtab" }`.

### Pocket interiors — the "TARDIS" pattern (bigger on the inside)

A small box you step into that opens onto a huge interior is just a **portal pair to a hidden
interior world** — no space-warping:
- the box's door (world A) → the interior's entrance (a *separate* world B);
- the interior's exit → **back** to the box (`exit: back`).
The interior world is **portal-only** reach (not in the menu, no direct link — reachable *only*
through its door). Why a separate world is the *right* model, not a hack: the interior can be **any
size**, independent of the tiny exterior ("bigger on the inside" is literally true), and it keeps the
**no-in-world-teleport** principle intact (you cross *between* worlds, not jump within one).

Emergent freebie: **many doors → one interior** — several boxes (even in different worlds) can target
the same interior; the back-stack returns each visitor to *their* door.

The "bit different" practicalities:
- **Seamlessness:** to sell the illusion the entry wants the **seamless handoff** (Phase 3); on the
  Phase-1 reconnect there's a brief load crossing the threshold.
- **Cost:** a box is small, but a full world/process per interior is wasteful → this is the strongest
  case for in-engine **pocket regions** (one process, several disjoint spaces; Phase 3), not a
  process per cupboard.
- **Authoring:** offer a one-step **"pocket room" preset** — place a doorway and it auto-creates the
  linked interior + the return portal (portal-only, exit-back) — so the operator doesn't hand-wire
  two portals and a world.

---

## 7. Reachability & access (per world)

Each world carries three independent reach toggles (the owner's "only via lobby / only direct"
cases are combinations):

| reach flag | Meaning |
|---|---|
| `menu` | Listed in the world picker. |
| `lobby` | A door/portal in the lobby leads here. |
| `direct` | A direct link connects here. |
| `portal` | Other worlds' portals may target here. |

Named cases:
- **Public** = `menu + lobby + direct (+ portal)`.
- **Lobby-only** = `lobby` only (not listed, no direct link).
- **Direct-only / private** = `direct` only (+ optional `access: signin|owner`).

**Two enforcement levels — same theme as the gallery's client-side vs server-side read-only:**
- **Unlisted (obscurity, cheap, Phase 1):** reachability is just what the manifest advertises and
  what doors exist. Someone who *learns* a direct address could still connect.
- **Enforced (Phase 2):** the destination world **rejects** connections that don't carry a valid
  **route ticket**. The lobby (or a portal) issues a short-lived signed ticket asserting "arrived
  via the lobby/portal"; a bare direct connect has none → refused. This is what makes "lobby-only"
  and "private" *real* rather than hidden. Same token also carries the return target.

Access (`open | signin | friends | owner`) is orthogonal to reach and reuses the existing
per-server access gates (require-signin / allowlist), now **per world**.

---

## 8. Data model (the manifest)

One file describes the whole Place. Console writes it; clients read it; the handoff layer enforces
it.

```json
{
  "place": { "name": "Ada's Place", "front_door": "lobby", "lobby": "hub" },
  "worlds": [
    { "id": "hub",     "title": "Lobby",          "type": "adventure",
      "reach": { "menu": true,  "lobby": false, "direct": true,  "portal": true },
      "access": "open", "exit": "out", "ws": "/ws/hub" },

    { "id": "gallery", "title": "Ada's Art",       "type": "gallery", "open_to_visitors": true,
      "reach": { "menu": true,  "lobby": true,  "direct": true,  "portal": true },
      "access": "open", "exit": "back", "ws": "/ws/gallery" },

    { "id": "maze",    "title": "The Maze",        "type": "adventure",
      "reach": { "menu": false, "lobby": true,  "direct": false, "portal": true },
      "access": "open", "exit": "lobby", "ws": "/ws/maze" },

    { "id": "studio",  "title": "Private Studio",  "type": "creative",
      "reach": { "menu": false, "lobby": false, "direct": true,  "portal": false },
      "access": "owner", "exit": "out", "ws": "/ws/studio" }
  ],
  "portals": [
    { "from": "hub",     "at": [10,65,10], "to": "gallery", "to_anchor": "spawn" },
    { "from": "gallery", "at": [4,65,20],  "to": "maze",    "to_anchor": [2,64,2] },
    { "from": "maze",    "at": [50,64,50], "to": "hub",     "to_anchor": "spawn" },
    { "from": "gallery", "at": [8,65,2],   "to": "url:https://ada.example/shop",
      "label": "Visit the shop", "confirm": true, "open": "newtab" }
  ]
}
```

(A world's `exit` can equally be an external target, e.g. the gallery's exit screen →
`"exit": { "type": "url", "href": "https://ada.example/shop", "label": "Visit the shop", "confirm": true }`.)

(In Phase 2 the `portals` array is generated from portal blocks placed in-world, not hand-edited.)

---

## 9. Admin panel (the "home admin panel")

The console is for **arrangement, not building** — it never edits world content (that comes from the
game, see §1). It grows from "configure one server" to **"arrange a Place"**:
- **Worlds:** the list of **published** worlds — re-order, rename, set **type** + **access** +
  **spawn**, remove. New worlds arrive by **publishing from the game** (not created here); re-publish
  from the game to update one. **No build/open toggle** (publishing *is* going live; visitors are
  read-only by nature — see §1).
- **Front door:** menu or lobby; choose which world *is* the lobby.
- **Per world:** the three **reach** toggles, the **access** level, and the **exit** policy.
- **Portals:** list / add / remove portal links (Phase 2: surfaced from in-world portal blocks).
- Per-world telemetry/moderation reuses the existing console panels, scoped per world.

The kind-of-place choice (gallery/creative/survival/adventure) is set when you **publish** a world
from the game, reusing the wizard's flow — not re-entered in the console.

---

## 10. Phasing (cheap → enforced → seamless)

**Phase 1 — Multi-world + discovery + client navigation (little/no engine risk).**
- **Publish from the game:** a "Publish to my server" action uploads a built world
  (operator-authenticated) to the Place as a served snapshot; re-publish overwrites (prior kept as a
  backup). This is how worlds get onto the server — none are built on it.
- Multiple worlds = **one engine process per world** behind Caddy path routing (orchestration +
  Compose). Console manages the manifest + arrangement.
- Client: fetch manifest → **world picker** + **front-door** handling; **back-stack**; **exit
  policy**; door = guided **reconnect** to another world's socket (carries client-held return).
- Reachability by **listing/obscurity**; access via existing per-world require-signin/allowlist.
- Delivers: many worlds from one URL, menu + lobby, remembered exits, public/unlisted/private,
  the fixed broken-lobby. Hub-and-spoke + simple doors.

**Phase 2 — Real portals + enforced reach (engine + protocol).**
- **Portal blocks** placed in-world; `portals` derived from them.
- **Handoff tokens** (signed, short-lived) carry identity + return target + route ticket →
  **enforced** lobby-only / private; return survives reconnect without relying on client memory.
- **Server-side access/edit enforcement** (shares the gallery read-only-server-side hardening and
  the web-edit-propagation work — see deps).
- Delivers: arbitrary world→world portals, truly private/lobby-only worlds.

**Phase 3 — Seamless + cross-Place (Spec 07 fleet).**
- Seamless traversal (preload destination, no visible reconnect); in-engine multi-world or fast
  handoff; **cross-Place portals** via a world directory / matchmaker; shared presence. This is the
  Spec 07 §8 intra/cross-shard/cross-world endgame.

---

## ✦ World assets & content safety (cross-cutting — galleries' images, etc.)

**Status: mechanism is design; thresholds/obligations are POLICY + LEGAL** (UK OSA / COPPA — see the
compliance research) and warrant their own spec + counsel before arbitrary image upload ships to a
child audience.

- **Assets live in the build environment.** A served world's extra content (a gallery's images/art;
  later, other per-type assets) is added **where you build it**, becomes part of the world, and is
  **published with it** (in the Stash bundle, pushed to the server). The console **drops its upload
  panel** → view/arrange-only. Kills the "build the world here, add the photos there" bounce.
  Separate the axes: **where** you add an asset = build env (UX); **when** it's checked = at publish.
- **Art is a personal library surfaced in the creative palette — not survival loot.** Images live in
  a personal **art library** (in your Stash, syncs across devices) and appear as a **"My Art" tab in
  the creative block-palette**, available only when **building (creative) in an exhibit-capable
  world** (galleries by default; optionally any creative world). Select an image → **place it as an
  exhibit** (wall art / billboard), reference-style (place it as many times as you like) — this
  **replaces the paste-a-`/exhibit`-command flow**. Images are **never in survival/adventure
  hotbars** (you don't carry art as loot). The **library** is all your art (the source you pick
  from); a world only contains the subset you **placed**, which travels with it on publish. Library
  is **per-game-namespaced** (your AxeNStax art doesn't bleed into other games). **RESOLVED (owner,
  2026-06-22): art lives in *gallery-labelled* worlds only** (any biome) — a normal world can't place
  images. ("Gallery" is a capability, not a rigid type — see §1.) This also contains the
  content-safety surface to gallery worlds. Content-safety scanning is **out of scope for now**
  (noted for awareness only).
- **Safety gate at the publish/serve boundary, not the upload UI.** Private→public is where the law
  and risk concentrate, and it's one chokepoint. On **publish of any world containing user images**,
  before it serves: **CSAM hash-match** (PhotoDNA/NCMEC-style) + a **classifier** pass
  (nudity/violence); fail → blocked. (CSAM carries mandatory NCMEC reporting — legal/counsel.)
  Published content is bound to the operator **npub** → accountability + takedown.
- **Child accounts are parent-gated** for uploading arbitrary images and publishing public worlds
  containing them (consistent with the parent-controlled model; identity defaults non-public). All
  publicly-served images are scanned regardless of account age.
- **Reduce reliance on arbitrary external upload:** prefer **in-game-created content** (builds,
  in-game screenshots) + **curated/approved asset packs**; arbitrary file-from-disk becomes the
  gated *exception* (for real artists), not the default.
- **Cross-game containment:** arbitrary-image-assets is a **per-game capability**, not an always-on
  engine feature; all enablers funnel through the *one* governed publish/serve safety pipeline, so a
  game that doesn't want it doesn't inherit the risk.
- **Encryption tension (state it deliberately):** if Stash is E2E-encrypted to the user, the platform
  can't scan at rest — fine if the scan is at publish (decrypt in client → scan → serve). So
  "private, unpublished" images are unscanned-but-undistributed; the public boundary is the scan point.
- **Honest current gap:** today's console upload (`studio.py`) validates only file format / magic
  bytes — **no content scanning** — and galleries can be public. Treat "**no public serving of user
  images without a content-safety gate**" as a **hard line** before this is promoted/marketed.

---

## 11. Dependencies & alignment

- **Stash (the content source):** worlds are built in the normal client, stored in your Stash, and
  **published from the game** to a Place; the server only serves snapshots. Stash already works
  in-game; publishing reuses it. This is why the **console doesn't need the blocked Stash-signer**
  wiring (an upstream Signet app change) — it only arranges already-published worlds. See §1.
- **Web-edit propagation — NOT on the critical path for this design.** Web clients can't send block
  edits to a *server* yet (native-gated; `2026-06-21-server-setup-wizard-design.md` §Gallery use
  case). But with publish-from-game you build in the *normal client* (where editing + saving work)
  and publish, so curated worlds never need it. It's only required for **live collaborative building
  on the server** (the separate capability in §1).
- **Gallery/showcase:** the build/open toggle is **replaced** by publish-from-game (draft→publish,
  §1). Showcase/kiosk read-only stays the Phase-1 client-side level for served worlds, with the same
  Phase-2 server-side enforcement.
- **Spec 07 §8 Portals & world lifecycle:** Phase 3 *is* that work, reached incrementally.
- **Rail/cart (rail-replaces-waystones):** unaffected — rail is *intra*-world transport; portals
  are *inter*-world links. Keep the boundary explicit.
- **Identity/access:** per-world access reuses attestation + require-signin + allowlist.

---

## 12. Open decisions (owner)

1. **Lobby-only / private enforcement:** ship Phase 1 as *unlisted* (obscurity) and add *enforced*
   route tickets in Phase 2 — or hold "private" claims until enforcement exists? (Recommend: ship
   unlisted, label it clearly as not-yet-enforced.)
2. **Multi-world mechanism for Phase 1:** confirm **one-process-per-world behind Caddy** (simple,
   isolated, some idle RAM/CPU per world) vs waiting for in-engine multi-world (Phase 3).
3. **Default front door** for a new Place: **menu** (self-serve, simplest) or **lobby** (curated)?
4. **Direct entrants & discovery:** may a direct-link visitor ever reach the menu (discover the
   rest of the Place), or only ever see the one world they were sent to?
5. **Portal authoring surface:** in-world portal blocks first, console-managed list first, or both?
6. **Idle worlds:** spin down empty world processes to save resources (cold-start on next visit)?
7. **External links (shop exits):** require a "you're leaving to `<domain>`" **confirmation**
   (recommend yes, especially for kids)? Open in a **new tab** (keep the experience) or **replace**?
   Native: **system browser**, **in-game QR/link panel**, or just fall back? And should outbound
   links — *especially shops / payment-adjacent ones* — be **disabled or gated for child accounts**
   per the platform's age-safety posture (COPPA / UK OSA — see the compliance research)?
8. **Asset upload & content safety (needs counsel):** move image upload into the **build environment**
   (recommended) + gate at **publish** with CSAM hash-match + classifier? Confirm **no public serving
   of user images without that gate** as a hard pre-launch line. How hard to lean on **in-game /
   curated assets** vs arbitrary upload? Child-account upload/publish **parent-gated**? This likely
   needs its **own spec + legal review** before build.
