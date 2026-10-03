# Server Setup Wizard — design

**Date:** 2026-06-21
**Status:** BUILT (this session) — console-only + entrypoint (no Rust/engine change)
**Owner ask:** A WordPress/OS-style first-run wizard for a dedicated server. On first
sign-in the operator is walked through "what kind of place is this?" (gallery vs
generic world vs …) in a few super-simple steps (an 8-year-old with a parent could do
it). Skipping is possible but deliberately hard (small text). A *findable* "re-run
setup" lives in the console afterwards. Then: rebuild the image and redeploy.

Related: `2026-06-18-operator-console-sidecar.md`, `2026-06-17-operator-console-design.md`,
`2026-06-16-dedicated-docker-server-design.md`, `2026-06-19-creator-gallery-showcase-design.md`.

## Where the wizard lives

The **Operator Console** (`tools/sites/console/`, FastAPI sidecar, reverse-proxied at
`/admin`) — the only place an operator signs in. The wizard is the console's first-run
route; zero game-client change.

The server **owner** is already "established at creation": it's the npub in
`<worlds>/.identity/attestation.json` (`pubkey`). The console only lets that npub (and
any roles it grants) sign in. The wizard does **not** create identity — it configures
the server the owner already owns.

## The hard constraint that shaped this

The engine reloads *some* settings live (~5s): name/about/region, announce, privacy,
allowlist/blocklist, require_signin, and the web kiosk's `showcase.json`. But **game
mode is baked into a world's metadata at creation** and the engine's showcase flag is
read at boot — neither is live-reloadable. So a wizard that can genuinely "create as a
gallery / creative world" must have the server *(re)boot into the chosen mode*.

We achieve that **without touching the Rust engine**, by making the container's
`entrypoint.sh` config-driven:

1. **First-run gate.** Caddy (hence `/admin`) starts immediately, but the engine
   **waits** until the operator finishes setup (the console writes
   `.identity/setup-complete`). So the operator's chosen game mode applies to the
   *first* world — no throwaway default world. `AXENSTAX_SKIP_WIZARD=1` disables the
   gate (boot on compose defaults).
2. **Config source.** Before launching the engine, the entrypoint sources
   `.identity/server.env` (shell-quoted `AXENSTAX_*` overrides the console wrote);
   these win over the compose defaults.
3. **Restart-watcher.** A `.identity/restart` sentinel makes the entrypoint cycle the
   engine (re-sourcing `server.env`). A `.identity/reset-world` sentinel makes it
   **archive** (rename aside, never delete) the current world so the engine recreates
   it fresh in a newly-chosen mode. Both are needed because the console runs in a
   *separate* container and can't `docker restart` the engine.

## Two independent "setup done" signals

- **Console "show the wizard?"** → `server.json.setup_complete` (in `.identity/`). Only
  the console reads it. `false`/absent → route the operator to `/setup`.
- **Engine "may I boot?"** → the `.identity/setup-complete` marker file. Only the
  entrypoint reads it. Present → release the gate.

True first run: neither exists → console shows the wizard, engine waits. Finishing the
wizard writes both. **Skip** writes the engine marker (so the box runs on defaults) but
leaves `server.json` marked not-complete-skipped, so the dashboard keeps nudging.

## The wizard steps (less is more)

1. **Welcome** — one line + **Start**. Tiny grey "skip setup for now" link.
2. **What kind of place is this?** (the one big choice)
   - 🏛️ **Gallery** — you build it and hang art; visitors look, can't change anything →
     `game_mode=creative` (so the **operator** can build) + kiosk capability. Starts in
     **build mode** (kiosk off). See "Gallery use case" below.
   - 🏗️ **Creative** — everyone builds with unlimited blocks → `game_mode=creative`.
   - ⛏️ **Survival** — gather, craft, survive → `game_mode=survival`.
   - 🧭 **Adventure** — explore & play, don't change the world → `game_mode=adventure`.
3. **Name it** — server name (prefilled) + one-line description (optional).
4. **Who can come in?** — Anyone / Signed-in players only / Just my friends
   (`require_signin` + optional npub list).
5. **Finishing touches** — "Show in players' server lists?" (announce) + "Keep a visit
   history?" (privacy `sessions:7` vs `none`).
6. **Review & create** — plain-language summary + **Create my server!**. If a re-run
   would change the game mode of an existing world, this step warns that a fresh world
   is started (old one archived).

Each step ≤ 3 questions; every choice has a safe default; jargon avoided.

## What "Create" writes (console → shared volume)

| Target | Written by | Effect |
|---|---|---|
| `showcase.json` | `studio.set_showcase_config` | web kiosk arms live (~next page load) |
| `console.json` | `identity.save_settings` | name/about/announce/privacy live (~5s) |
| `require_signin` | `identity.set_require_signin` | access gate live (~5s) |
| `whitelist.txt` | `identity.add_to_list` | friends list live (~5s) |
| `server.env` | `wizard.write_env` (shlex-quoted) | engine boot config (game mode, showcase, name…) |
| `server.json` | `wizard.save_state` | the console's own record + re-render |
| `setup-complete` | `wizard.mark_engine_ready` | releases the engine gate |
| `restart` / `reset-world` | `wizard.request_restart` | only on a re-run that changes game mode / forces fresh |

## Reset / re-run (findable)

The dashboard shows a **"Re-run setup wizard"** button (and a banner if setup isn't
finished). It sets `server.json.setup_complete=false` and routes to `/setup`. Re-running
and **keeping** the world applies live settings only. Re-running and **changing the game
mode** (or ticking "start a fresh world") archives the current world and reboots into
the new mode via the `reset-world`+`restart` sentinels.

## Safety / robustness

- The entrypoint never **deletes** a world — only renames it `…​.archived-<ts>`.
- Gate failure mode: if the gate logic misbehaves the worst case is "engine waits";
  `AXENSTAX_SKIP_WIZARD=1` is the documented escape hatch, and prior images are tagged
  `:backup-<date>` before redeploy for rollback.
- Backwards compatible: with `setup-complete` present (or `AXENSTAX_SKIP_WIZARD=1`) and
  no sentinels, the entrypoint behaves exactly as before.

## Gallery use case (the operator builds; visitors look)

The contradiction to avoid: a gallery built as pure **adventure** is read-only for
*everyone, including the operator* — so the operator could never build it. (This bit a
real first user: "Ada's Art" was created adventure and couldn't be built; that's what
prompted this revision.)

**Model (verified against the engine, no engine change):**
- A Gallery is a **creative world** → the operator can place blocks and hang art.
- **"Open to visitors"** arms the web kiosk (`showcase.json.enabled=true`). The web
  client *forces* `PlayMode::Adventure` (read-only) whenever the kiosk is on, **regardless
  of the world's game mode** (`game_loop.rs` — confirmed). So creative-world + kiosk-on =
  web visitor can only look + collect exhibits.
- A gallery therefore **starts in build mode (kiosk off)**, and the dashboard offers a
  one-click **"Build mode ⇄ Open to visitors"** switch (`/api/gallery/visitors` →
  `studio.set_showcase_config`). Flipping it **never changes the world or restarts** — the
  gallery's contents are always safe.

**Timing nuance:** `showcase.json` is read at **page load** on the web client (no live
poll). So the switch applies to a visitor **the next time they open the link**, and the
operator should **reload their own game tab** after switching (build⇄open) to see it.

**Known limitations (documented honestly in the console + README):**
- Read-only is enforced for the **web link** (the visitor audience) and is **client-side**
  — the dedicated server does **not** yet gate block edits by game mode (confirmed: no
  server-side edit authorisation). A **native** client joining a creative gallery could
  still edit. Mitigation offered in the UI: set *Access → friends only*.
- **Future hardening (engine work, deferred — see memory `play_modes_shipped` per-player
  note):** server-authoritative read-only via either (a) a server-side edit gate keyed on
  game mode, or (b) per-player game mode so verified operators get creative and everyone
  else the world default (adventure) — then no build/open toggle is needed at all.

## Use-case guide (dashboard)

The dashboard adapts to `server_type` with a plain-language hero ("the admin helps people
think"): Gallery shows the Build/Open switch + share link + "your art is below"; Creative
/ Survival / Adventure each get a one-line "what this means for your players". For a
gallery the **images (Studio) panel is featured full-width** and the duplicate World/
showcase panel is hidden (the hero owns the switch).

## Deploy

Console image rebuilt (pure Python). Server image rebuilt for the new `entrypoint.sh`
**reusing the already-built engine binary + web bundle** (staged from the running
container — no cargo/trunk). Old images tagged `:backup-*`. The pre-existing dev world
is archived so the morning test is a clean first-run; the operator identity
(attestation/admins) is left intact so sign-in still works.
