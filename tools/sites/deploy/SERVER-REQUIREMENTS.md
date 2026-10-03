# What a server needs to run Axe'n'Stax

The required / optional / removed inventory for standing the public stack up.
Pairs with the cutover steps in `README.md`. Decisions current as of 2026-06-06.

## TL;DR

- **Run:** the **seven FastAPI sites** + the **WASM game bundle** + **Caddy**. That's it.
- **Need at build time:** Rust + trunk (WASM bundle), Python 3.11 + venv, `libudev-dev`.
- **External (browser-side):** the Signet relay for sign-in; Primal's Blossom for cloud save.
- **Do NOT spin up:** the voice server, Node/npm, or the Groq/OpenAI keys. **Removed.**

## Sites — all WANTED (not deferred)

All seven ship and are part of the stack:

| Site | Port | Host | Notes |
|---|---|---|---|
| Marketing | 8096 | `axenstax.com` | product front door |
| Game | 8094 | `play.axenstax.com` | PWA runtime + WASM bundle |
| Learn | 8098 | `learn.axenstax.com` | guided journey |
| Wiki | 8097 | `wiki.axenstax.com` | player reference |
| Claim | 8100 | `claim.axenstax.com` | merch fulfilment; shows "not open yet" until a Printful token is set — safe to run without it |
| Project | 8099 | `axenstax.org` | open-source home |
| Docs | 8095 | `docs.axenstax.org` | specs / ADRs / roadmap / native build |

Each is a uvicorn service (systemd unit) behind Caddy. Python deps in each
`requirements.txt`. The only one carrying a secret is **game** (`NOSTR_SERVER_KEY`,
can be ephemeral) and **claim** (`PRINTFUL_TOKEN`, optional).

## REQUIRED

| Thing | For |
|---|---|
| The 7 FastAPI sites (uvicorn + systemd) | the public stack |
| WASM game bundle (`trunk build` → `game/engine/dist/`) | the actual game on `play.` |
| Caddy (`axenstax.Caddyfile`) | reverse proxy + auto-TLS |
| Signet relay `wss://relay.trotters.cc` *(external, our infra)* | player sign-in |
| Rust + cargo + `wasm32` target + `trunk` *(build host)* | building the WASM bundle |
| Python 3.11 + `python3.11-venv` *(host)* | running the sites |
| `libudev-dev` *(build host)* | engine build (gilrs gamepad) |
| **Primal Blossom** `https://blossom.primal.net` *(external)* | **web/PWA cloud save** — set as `BLOSSOM_PUBLIC_URL` in `env.game.template` |

## OPTIONAL / DEFERRED

| Thing | State |
|---|---|
| `PRINTFUL_TOKEN` (claim site) | blank = claim shows "not open yet"; owner sets it later |
| `NOSTR_SERVER_KEY` (game) | blank = ephemeral (regenerated per restart, warns) |
| Native desktop binary | **build-from-source** (Linux/Win/Mac) — see the docs `/download` page. A pre-built Linux binary is an optional convenience. |
| **Native cloud save** | **NOT built (a feature gap, not a build bug).** The native binary compiles fine; it just has no Blossom path and saves locally (no error). Cloud save needs a **signed** Blossom upload + a Nostr manifest + a signer — the web gets the signer from Signet in the browser; **native has no Nostr sign-in/signer at all** (same gap as native multiplayer, Spec 1 Phase 4). So "native uses Primal Blossom" is gated on **native sign-in**, not a rebuild. `BLOSSOM_PUBLIC_URL` only reaches the web client. |
| Bitcoin / Lightning / phoenixd | deferred |

## REMOVED — do not provision

The voice subsystem is **out of the stack** (typed beats voice for kids):

| Thing | Why removed |
|---|---|
| Voice server (Node, `:4100`, `tools/voice-server/`) | served the Games Master voice widget + voice-feedback intake — both dropped |
| Node.js + npm | only the voice server needed them |
| `GROQ_API_KEY`, `OPENAI_API_KEY` | only for Whisper / triage / TTS |
| Games Master widget | removed (hardcoded `:4100/widget.js` script tag stripped; `VOICE_SERVER_ORIGIN` defaults blank) |
| `AxeNStax-internal` repo (feedback storage) | only the old voice-feedback wrote there |

**Feedback today:** the in-game **voice recorder + admin board are removed** —
`feedback.js`, `feedback-admin.js`, `feedback_admin.py`, the two board templates, and
the `<script src="/static/feedback.js">` in `game/engine/index.html` are all gone, and
`/api/feedback` returns a graceful "being rebuilt" 503. The replacement is **typed
feedback published to a dedicated internal Nostr inbox** — config slots reserved in
`env.game.template` (`FEEDBACK_NPUB` / `FEEDBACK_NSEC` / `FEEDBACK_RELAY`), awaiting the
owner's keys.

**Residual dead code (safe, flagged for the engine session):** two small bits remain
because deleting them cleanly needs an engine rebuild + edits to `main.rs`, which the
other active session is currently in:
- the `/api/feedback` route's old proxy body + `_FEEDBACK_*` constants in `game/app.py`
  (unreachable — the 503 fires first), and
- ~~the engine snapshot hooks `wasm_feedback.rs` + `feedback_log.rs`~~ — **deleted
  2026-10-01** with the rest of the web feedback channel (`feedback_log.rs` survives
  as `web_logger.rs`, console-only).

> Supersedes the "Start the voice server (VM)" section of the root `CLAUDE.md` —
> that section is now stale.
