# Domain & Site Information Architecture

**Status:** Decided + **implemented repo-side** (2026-06-06). The six sites now
exist under `tools/sites/` (marketing, game, learn, wiki, project, docs) with
their Caddyfile, systemd units, and env templates. **Not yet live** — the
box-side cutover (new DNS + units + Caddy vhost reload) is pending; the ordered
runbook for the box operator is `tools/sites/deploy/README.md`. Phase-2 content
work still open: re-voice the `learn` journey, flesh out the `axenstax.org`
project home, switch on `SOURCE_URL` when the repo goes public. The AI-consumers
appendix below remains exploratory.

**Owner driver:** Staxolottle wants the public-facing structure rethought. The
current live layout grew out of "we don't own `axenstax.app`, so the game landed
on the bare `.com`," which left marketing exiled to `.org` and a login wall as
the front door.

## The organising principle

**One brand, two domains, split by audience — not by deployment.**

- **`axenstax.com` = the product.** For people who want to *play*: players, kids,
  press. "I want to use the thing."
- **`axenstax.org` = the open-source project.** For people who want to *build*:
  contributors, self-hosters, spec readers. "I want to understand / run / extend
  the thing."

This is the WordPress model (`wordpress.com` = hosted product, `wordpress.org` =
open-source self-host) and the Mozilla model (`.org` is the project/community
home). The `.org` TLD has signalled "community / open-source" for decades; using
it for the project side is the convention working as intended.

The single line that decides where anything goes:

> **Use the product → `.com`. Build the product → `.org`.**

The same line draws every sub-boundary below (wiki vs docs, client install vs
server install).

## Target layout

### `axenstax.com` — the product (for players)

| Hostname | Role | In web nav? |
|---|---|---|
| `axenstax.com` | Marketing front door — pitch, screenshots, single primary CTA: **"Play"** → `play.` | Yes (root) |
| `play.axenstax.com` | The game (PWA runtime) — fills the missing `.app`; **install-as-PWA** happens here (browser prompt) | Yes |
| `learn.axenstax.com` | **Guided journey** — light, warm, kid/parent-friendly onboarding ("be shown around") | Yes |
| `wiki.axenstax.com` | **Reference** — dense, lookup-driven player guide ("look something up") | Yes |

**`learn` vs `wiki` — two different modes, deliberately split:**

- **`wiki` = reference.** Dense, lookup-driven. "Tame a wolf," "smelt iron," "what
  does this block do." You *dip in*. Feels like a wiki — the word signals it.
- **`learn` = guided journey.** Light, narrative, warm, kid- and parent-friendly.
  You don't *consult* it, you're *taken through* it. Tonally it's marketing's
  friendly cousin, **not** the wiki's sibling — it sits between the pitch and the
  reference. Keep it markety, never instructional. ("tutorials" was rejected as a
  name precisely because it reads technical/instructional.) The verb `learn` was
  chosen because it's the one word that stays cleanly on the *learn-about* side of
  the line — distinct from `play` (the *do* layer); brand-native game verbs
  (`dig`/`build`/`craft`) were ruled out for colliding with play. `quest` was
  liked but **deliberately reserved for in-game content, not a subdomain** —
  quests are gameplay (you go *on* one, you don't browse to one). They are the
  *hands-on* half of the guided journey (see the in-game feature-coverage
  challenges, `docs/foundations/2026-06-06-feature-coverage-challenges.md`):
  `learn` teaches *about* (out-of-game, read/watch), in-game **Quests** teach *by
  doing*. The two reinforce each other and step on neither `play` nor `wiki`.

**`learn` is content, not a destination — the delivery ladder (decided 2026-06-06).**
The owner reframed `learn`: it's a **companion** used *alongside* the live game,
not a brochure you read first (the steps assume the game is in front of you). The
durable asset is the **content itself, written once as clean markdown** in the
repo (next to the engine, same source-of-truth discipline as the specs). That one
source flows to many *delivery surfaces*, cheapest first — **write once, read by
anything:**

| Surface | Effort | Notes |
|---|---|---|
| **Web page** (`learn.axenstax.com`, phone or desktop) | ~free | The companion *today*: open it on the phone they already signed in with, prop it beside the screen/TV. Needs only good phone-first CSS. |
| **PDF / printout** | minimal | Repurposes the same markdown; reuses the existing docs print pattern (`/print/...` + `print-sheet.js`). Paper beside the screen. |
| **External / BYO AI assistant** | ~free | Kid or parent asks their own ChatGPT/Claude/Gemini "how do I … in Axe'n'Stax" → it reads the markdown and guides them. See AI appendix §3. |
| **In-game Guide tab** (egui overlay) | moderate | The engine renders the *same markdown* natively (e.g. `egui_commonmark`), bundled with the build → offline, cross-platform, **no second device**. Open / read / close / play. (A real web page can't be embedded in the painted game canvas; the in-game UI is egui, not a browser. A web-only iframe-over-canvas hack exists but breaks on native.) |
| **In-character NPC walkthrough** | north star | An in-world guide character. Real engineering + the AI-companion idea. Deferred. |

Implication for the marketing CTA: because `learn` *needs the game*, "Learn first"
as a separate pre-game door is the wrong frame — `Play` stays the clear primary;
the guided experience is entered from/alongside play, and a quieter secondary is a
light "how it works" rather than a path that sends people away from the game.

### `axenstax.org` — the open-source project (for builders)

| Hostname | Role | In web nav? |
|---|---|---|
| `axenstax.org` | Project front door — "open-source voxel engine, self-hostable," GitHub, contributor pitch | Yes (root) |
| `docs.axenstax.org` | Engine specs (01–08), ADRs, architecture, roadmap — **how it's built** | Yes |
| self-host area (on `.org`) | Server binary, Docker image, build-from-source, run-your-own-server guide | Yes |

### Machine-facing endpoints (the game dials these — never in any nav)

| Hostname | Role |
|---|---|
| `server.axenstax.com` *(reserved, dark)* | Multiplayer connection endpoint — typed into the game, not a browser |
| `api.` / `mm.` *(reserved)* | Matchmaking / session API when multiplayer ships |

## "Install" today = PWA, not a binary download

**The install path we promote is installing the PWA**, not downloading an app.
There are no native "apps" to get yet, and the only binary that exists is a Linux
x86-64 dev build. So for alpha:

- **Primary CTA everywhere is "Play"** → `play.axenstax.com`. The game runs in the
  browser (Chromium, PWA-first per ADR-003 / the PWA-priority posture).
- **Installing is a browser action on `play.`**, not a download: the PWA install
  prompt / "Add to Home Screen" (manifest + service worker already shipped on the
  game site). "Install to your device" is the wording, and it happens *in* `play.`
  — **do not** put "Download for Windows / Mac / Linux" on the front door; it
  implies binaries we aren't shipping.
- **Native binary installers (`.exe`/`.msi`/`.dmg`/`.deb`/AppImage) are DEFERRED.**
  When they exist, a "Get the app" download can join `.com` next to Play. Until
  then the lone Linux dev binary stays low-key on the builder side (`/self-host`
  on `.org`), unpromoted.

So the only download that's relevant *now* is the **builder** one (server binary /
Docker / source → `.org`). The **player** download row below is a *future* shape,
switched on when real installers ship:

| Download | Who wants it | Side | Status |
|---|---|---|---|
| **Install the PWA** (browser prompt) | A **player** | `play.` (`.com`) | **Live — the promoted path** |
| **Native client** — `.exe`/`.dmg`/`.deb`/AppImage | A **player** who wants native | `.com` "Get the app" | **Deferred** (no installers yet) |
| **Server build / Docker / source** | A **self-hoster / dev** | `.org` `/self-host` | Lift-and-shift |

## Migration from what's live today

| Today (live) | Target |
|---|---|
| `axenstax.com` = game login wall | `axenstax.com` = marketing; game → `play.axenstax.com` |
| `axenstax.org` = marketing landing | `axenstax.org` = open-source project home (**new content needed**) |
| `docs.axenstax.com` = wiki + tutorials + specs + internal, all in one nav | Splits 3 ways: reference → `wiki.axenstax.com`; tutorials → re-voiced as the guided journey on `learn.axenstax.com`; specs/ADRs/roadmap → `docs.axenstax.org` |
| Internal / "Claude's working docs" publicly browsable | Drops to repo-only — off the public sites |
| Native download under docs | Player client → `.com`; server/source → `.org` |

Redirects to preserve so links don't rot:
- `docs.axenstax.com` → `docs.axenstax.org`
- `www.axenstax.com` → `axenstax.com`, `www.axenstax.org` → `axenstax.org`
- whatever old `axenstax.org` marketing URLs people have bookmarked → `axenstax.com`

## What's genuinely new build work

1. **`axenstax.org` project home** — does not exist. A real project / self-host
   front door (what the engine is, why custom, how to run a server, the docs).
   **Repo is private for now (opening soon)** — so it ships docs-forward, with the
   public-GitHub / "contribute" surface staged to switch on at open-sourcing (see
   Decision 2). Can start minimal and grow.
2. **`wiki.axenstax.com` split-out** — the player *reference* moves out of the
   docs site to its own hostname (mostly a lift-and-shift of the player guide).
3. **`learn.axenstax.com` guided journey** — the current tutorials are not just
   moved but **re-voiced**: lighter, warmer, narrative, markety — a guided
   journey, not a manual. This is new tone work, not a copy-paste.

Everything else is moving an existing site to a new hostname + Caddy vhost +
redirects. The `server.`/`api.` endpoints are reserved but answer nothing until
multiplayer ships (deferred per the alpha launch posture).

## Decisions

1. **Bare `axenstax.com` = marketing front door. DECIDED 2026-06-06.** The game
   moves to `play.axenstax.com`; the root becomes the pitch + "Play" / "Get the
   app." No login wall at the front door.
2. **`axenstax.org` = project-docs home now, full open-source hub when the code
   opens. DECIDED 2026-06-06.** The source repo is **currently private** (GitHub
   `decented/axenstax`), with **open-sourcing planned soon**. So `.org` launches
   carrying what *can* be public now — engine specs, ADRs, architecture, roadmap,
   self-host docs — and the *source-browse / contribute* surface (public GitHub
   link, "contribute" CTA) switches on when the repo goes public. **Until then,
   do not surface a public "Source"/GitHub link that 404s for outsiders** (the
   current marketing footer link to the private repo should be held or pointed at
   the `.org` project page instead). Build `.org` so flipping on the GitHub link
   is a one-line change the day the repo opens.

3. **Install = PWA now; downloads stay as routes, no dedicated host. DECIDED
   2026-06-06.** The promoted install path is the **PWA install on `play.`**, not
   a binary. Native client installers are **deferred** (none exist) — when they
   ship, a `/download` "Get the app" page joins `.com`. Builder artefacts (server
   / Docker / source) → a `/self-host` page on `.org`. **No `get.`/`dl.`
   subdomain** — premature; revisit only if download complexity (multi-OS,
   version matrices, mirrors/CDN) actually materialises. Low-stakes, reversible.

### Still open

4. **Redirect strategy** — mechanical, not a real decision: standard Caddy at
   cutover. `docs.axenstax.com` → `docs.axenstax.org`, `www.*` on both, old
   `.org` marketing URLs → `.com`.

---

## Appendix — AI as a new class of consumer (EXPLORATORY)

> Captured 2026-06-06 at the owner's prompt. §1 (AI that plays) and §2 (AI that
> builds) remain **exploratory** — a map to react to, not a plan. **§3 (AI that
> reads) was promoted to NEAR-TERM, NEAR-FREE on 2026-06-06** — it's the
> external/BYO-AI rung of the `learn` delivery ladder and rides decisions already
> made (clean public markdown).

The key insight: **AI consumers mirror the human split.** They don't break the
`.com`/`.org` model — they map onto it.

- **AI that *plays*** (acts in the world) → product side / machine-facing
  endpoints. Same bucket as the multiplayer server.
- **AI that *builds*** (writes mods, generates content, runs servers) → project
  side / `.org`.
- **AI that *reads*** (answers questions about the game, represents it) →
  cross-cutting: clean, machine-readable content on every public site.

### 1. AI that plays / inhabits the world (product side)

- **Companion / NPC agents** — AI-driven characters that live in the world: a
  build buddy, a guide, genuinely-behaving villagers. (The voice-server "Games
  Master" widget is an early shadow of this.)
- **Tutor agent** — an in-world AI that teaches a kid to play, live and
  contextual. Natural fit with the existing voice stack.
- **Player-surrogate / bot** — AI that plays on your behalf (auto-farm, continue
  your build). **⚠ Direct tension with anti-bot + proof-of-play** — see below.
- **Opponent / co-op AI** — AI players in minigames (Hash Dash, Satori Rush),
  adaptive difficulty, filling out small servers.

**Needs:** a way for an agent to connect to a world the way a client does — a
programmatic/bot connection (machine-facing, the `server.`/`agents.` bucket).

### 2. AI that builds / extends the game (project side, `.org`)

- **Mod-writing agents** — an AI reads the modding/engine spec and emits a WASM
  plugin. (The one the owner named.) Needs machine-readable API reference,
  schemas, examples.
- **Content-generation agents** — worlds, structures, quests, recipes, textures
  from prompts. The Workshop (Spec 40) reskin/reshape loop and the image-gen
  pipeline are already AI-adjacent.
- **Server-ops agents** — AI co-admins that run / scale / moderate a server.
  Directly serves the "creators run monetised servers" vision.

**Needs:** machine-readable specs, a modding API, and likely an **MCP server**
(`mcp.axenstax.org`) and/or programmatic API exposing docs + modding surface to
coding agents.

### 3. AI that reads / represents the game (cross-cutting — NEAR-TERM, NEAR-FREE)

This is the cheapest and highest-leverage AI surface, and it's **near-term, not
exploratory** (sharpened 2026-06-06). You don't build a chatbot — **the kid and
the parent bring their own** (ChatGPT / Claude / Gemini on the phone already in
their hand from signing in). Your only job is to be the **best possible thing for
that AI to read.** A kid says "go to learn.axenstax.com and tell me how to make a
workbench" → the assistant reads the markdown and guides them. It's the
**external/BYO-AI rung of the `learn` delivery ladder** (see above) and it falls
out of decisions already made — clean markdown is *the* ideal LLM format.

- **The baseline is free** because the guide is clean public markdown. The cheap
  additions that make it *reliable* rather than accidental:
  - **`llms.txt`** at each public site root — a sitemap *for* LLMs listing the
    guide/wiki/spec pages. Tiny file.
  - **Expose raw `.md`** (not just rendered HTML) so an assistant gets clean
    source — a small route, optional.
  - **Self-contained pages** — each page stands alone (an AI may pull just one):
    clear title, clear steps, no "as we said above." A *writing* discipline.
- **The real cost is content quality, not engineering.** Because an AI will
  **faithfully repeat the guide to a child**, the markdown must be **accurate**
  (it's the ground truth the AI parrots) and **current** (a stale guide makes the
  AI confidently wrong). Living in the repo next to the engine keeps it in sync.
- **Official "ask the docs" agent / MCP** — `mcp.axenstax.org` exposing the wiki
  + specs as tools — is the *heavier, later* option. The near-term win needs none
  of it: just clean public markdown + `llms.txt`.

This applies across `learn` (player journey), `wiki` (player reference), and
`docs` (builder specs) — all already markdown.

### Two cross-cutting tensions worth deciding early

1. **AI-that-plays vs anti-bot + sats.** Proof-of-play pays real Bitcoin and
   Spec 08 has anti-bot precisely to stop bots farming sats. "AI that plays"
   forks into *sanctioned* agents (companions, your own agent on non-Bitcoin
   servers / sandboxes) vs *adversarial* bots (sat-farming). This is a policy
   line, not just a feature.
2. **Agent identity is already a Signet primitive — not new machinery.** Signet's
   current protocol (`forgesworn/signet/spec/protocol.md` §17 — the live source;
   the old `signet-protocol` repo is stale/superseded, do not cite it)
   already distinguishes humans from agents with three orthogonal mechanisms:
   - **Entity type** — `natural_person` / `persona` (human) vs `personal_agent` /
     `organised_agent` / `unlinked_agent` (agent). Agents are created by
     *delegation* from a verified operator and inherit/lose the operator's trust.
   - **`["agent-type", "ai"|"human"|"device"]`** tag — marks an AI vs a human
     delegate vs a device.
   - **`["mode", "teleoperated"|"autonomous"|"assisted"]`** — per-message "who is
     driving right now," explicitly an anti-deepfake / honest-AI signal.

   So AxeNStax's "operator controls nested agent accounts" *is* Signet's
   operator-delegation model — nothing to build, and per the Signet boundary rule
   nothing to ask Signet for. The anti-bot/sats fork (tension 1) can key off real
   protocol fields: e.g. sats-earning requires `natural_person`; agents earn only
   under explicit operator policy. **Caveat:** this is protocol spec — the live
   `signet-login` SDK AxeNStax consumes doesn't surface even persona/NP yet, so
   agent-type/mode consumption is further out than the protocol implies. The
   existing `np_flag` is the natural-person-vs-persona (human privacy) axis, a
   *different* axis from human-vs-bot.

### Candidate new reservations (if AI consumers are pursued)

| Hostname / file | For | Side |
|---|---|---|
| `mcp.axenstax.org` | MCP server — docs/specs/modding API to coding agents | project |
| `api.axenstax.org` | programmatic modding / automation API | project |
| `agents.axenstax.com` (or fold into `server.`) | in-world AI agent connection endpoint | product / machine-facing |
| `/llms.txt` on each public site | AI readers / discovery | cross-cutting (a file, not a host) |
