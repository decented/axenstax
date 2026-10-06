# Axe'n'Stax — site layout

Seven small FastAPI apps (six public sites plus the isolated `claim/` intake), one per hostname, split by **audience**:
**`.com` = the product** (for players), **`.org` = the open-source project**
(for builders). Locally each app lives on its own port. The full rationale and
the migration from the previous layout is in
`docs/architecture/2026-06-06-domain-and-site-architecture.md`. The box-side
cutover steps are in `deploy/README.md`.

## Layout

| Site | Dir | Port | Production host | Role |
|------|-----|------|-----------------|------|
| **Marketing** | `marketing/` | 8096 | `axenstax.com` | Product front door — the pitch, single "Play" CTA |
| **Game** | `game/` | 8094 | `play.axenstax.com` | `/game`, WASM bundle, install-as-PWA. An anonymous, login-free offline taster: no Signet auth, no cookies, no feedback channel (the voice server was removed 2026-06-23) |
| **Learn** | `learn/` | 8098 | `learn.axenstax.com` | Guided journey — light, warm onboarding (re-voice pending) |
| **Wiki** | `wiki/` | 8097 | `wiki.axenstax.com` | Player reference — dense, lookup-driven |
| **Claim** | `claim/` | 8100 | `claim.axenstax.com` | Merch fulfilment intake — code-gated, no payment (see `claim/README.md`) |
| **Project** | `project/` | 8099 | `axenstax.org` | Open-source project home (links to the public source repo) |
| **Docs** | `docs/` | 8095 | `docs.axenstax.org` | Engine specs, ADRs, roadmap, self-host download |

`.com` group = marketing + game + learn + wiki + claim. `.org` group = project + docs.

## Spin up

All seven (the six public sites plus `claim/`):
```bash
./start-all.sh
```

Stop all seven:
```bash
./stop-all.sh
```

One at a time:
```bash
cd wiki && ./start.sh        # or marketing/, game/, learn/, project/, docs/
```

Each `start.sh` kills a stale `python app.py` on its own port, refuses if
something unrelated holds the port, and creates a `.venv` on first run.
Logs land at `/tmp/axenstax-{marketing,game,learn,wiki,project,docs,claim}.log`.

## Inter-site links

Each app's templates pick up the other origins from env vars so links point at
the right host in production. `start-all.sh` exports all six locally:

| Var | Default | Read by |
|-----|---------|---------|
| `GAME_URL` | `https://localhost:8094` | marketing, learn, wiki, docs, project |
| `DOCS_URL` | `https://localhost:8095` | marketing, project |
| `MARKETING_URL` | `https://localhost:8096` | game, learn, wiki, docs |
| `WIKI_URL` | `https://localhost:8097` | marketing, project |
| `LEARN_URL` | `https://localhost:8098` | marketing, project |
| `PROJECT_URL` | `https://localhost:8099` | marketing |
| `SOURCE_URL` | *(blank locally)* | marketing, project — public GitHub link; set in the prod env templates |

Production values live in each site's `.env` (built from
`deploy/env.<site>.template` on the box). `SOURCE_URL` is set to the public
repo in the marketing and project templates (a blank value renders "Source (soon)").

## Why one app per host, not one app with subdomain routing

The split is by *audience* and *trust boundary*:

- **Game** serves the WASM bundle and the PWA shell — strict CSP, hashes pinned.
  It sets no session cookie and has no `/auth/*` routes (the web build is
  login-free; `test_no_login.py` pins that). Native sign-in lives in the engine.
- **Docs / wiki / learn / project** are read-only content — no cookies, no auth,
  no JS deps beyond markdown rendering. Compromise = vandalism.
- **Marketing** is near-static promotion — tightest CSP, no scripts.

A single app with domain-routed middleware would re-merge those trust zones in
code. Separate apps keep them mechanically isolated, and each gets its own
systemd unit + Caddy vhost.

## venv portability note

If `python3 -m venv` fails on a host (e.g. Debian/Ubuntu without
`python3.12-venv`), clone an existing working venv into the new sites:

```bash
for d in marketing game learn wiki claim project docs; do
    cp -a game/.venv "$d/.venv" 2>/dev/null || true
done
```

The `pyvenv.cfg` paths resolve via `home = /usr/bin`, so a copied venv runs fine
on the same host.

## Tests

`./check.sh` at the repo root runs every site's Python tests: `docs` on a bare
`python3`, and `console` (plain scripts), `game` and `marketing` (pytest) each in
their own `.venv`. `game` and `marketing` keep their test-only dependencies in
`requirements-dev.txt` (never installed on the deploy box). A missing venv fails
the gate and prints the install line; a new site with a `test_*.py` must be
registered in `check.sh`'s `site_tests` list.
