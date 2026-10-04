# Production deploy + cutover — Axe'n'Stax sites

Deploys **six** sites to the production host (`DEPLOY_HOST` below — a Debian 12
VPS). The box is shared with other services — **every change here is additive;
never touch another tenant's dir, unit, or Caddy vhost.**

> **This file is the box-side runbook.** The *what/why* of the structure is
> `docs/architecture/2026-06-06-domain-and-site-architecture.md`. This is the
> *how* — the ordered steps to stand the new layout up on the box.

## The new layout

Audience split: **`.com` = the product** (players), **`.org` = the open-source
project** (builders). Each public host is its own loopback uvicorn service:

| Host | Service / port | Dir |
|------|----------------|-----|
| `axenstax.com` | `axenstax-marketing` :8096 | `marketing/` |
| `play.axenstax.com` | `axenstax-game` :8094 | `game/` |
| `learn.axenstax.com` | `axenstax-learn` :8098 | `learn/` |
| `wiki.axenstax.com` | `axenstax-wiki` :8097 | `wiki/` |
| `claim.axenstax.com` | `axenstax-claim` :8100 | `claim/` |
| `axenstax.org` | `axenstax-project` :8099 | `project/` |
| `docs.axenstax.org` | `axenstax-docs` :8095 | `docs/` |

**This is the INVERSE of the old layout** (old: `axenstax.com` = game,
`axenstax.org` = marketing, `docs.axenstax.com` = everything). The three existing
units (`game`, `docs`, `marketing`) are **reused** — same ports — but get new
`.env` (cross-site URLs) and Caddy re-points which host hits which port. Three
**new** units (`learn`, `wiki`, `project`) are added.

## Artefacts in this directory

| File | Installed to | Purpose |
|------|--------------|---------|
| `axenstax-{game,docs,marketing,wiki,learn,project,claim}.service` | `/etc/systemd/system/` | uvicorn services, `User=deploy`, loopback ports |
| `axenstax.Caddyfile` | `/etc/caddy/conf.d/` | reverse-proxy vhosts; imported by main Caddyfile; auto-TLS |
| `env.{game,docs,marketing,wiki,learn,project,claim}.template` | rendered to each site's `.env` **on the box** by `render-env.py` | prod env; **no secrets** — host blanks preserved across deploys (claim needs `PRINTFUL_TOKEN`) |
| `render-env.py` | run on the box by `remote-update.sh` | merges a template with the existing host `.env`: template wins for config, blanks never clobber host secrets, `NOSTR_SERVER_KEY` auto-generated |
| `remote-update.sh` | run on the box by CI | **self-provisioning**: venv + `.env` + unit install/refresh + restart + Caddy vhost install on a fresh box only (a live vhost is host-owned and never overwritten). Idempotent; stands the layout up from scratch |

## Access

```bash
ssh -i ~/.ssh/<deploy-key> deploy@DEPLOY_HOST
```

The deploy key is non-default-named, so `-i` is required. CI uses the same key
from the `HETZNER_SSH_KEY` repository secret.

## DNS (Cloudflare — do this FIRST)

All hostnames are plain **A → DEPLOY_HOST, DNS-only (grey cloud)** so Caddy can
issue Let's Encrypt. No stray AAAA. Records needed:

| Record | Status |
|--------|--------|
| `axenstax.com`, `www.axenstax.com` | already exist |
| `play.axenstax.com` | **NEW** |
| `learn.axenstax.com` | **NEW** |
| `wiki.axenstax.com` | **NEW** |
| `claim.axenstax.com` | **NEW** |
| `axenstax.org`, `www.axenstax.org` | already exist |
| `docs.axenstax.org` | **NEW** |
| `docs.axenstax.com` | keep — now 301s to `docs.axenstax.org` |

Add the five NEW records (grey-cloud) and let them resolve before reloading Caddy,
or ACME issuance for those hosts fails.

---

## The supported path: push to main

DNS aside (Cloudflare, above — do that once by hand), **the cutover and all
ongoing deploys are automatic**: `remote-update.sh` (run by CI on every push to
`main`) self-provisions every site — venvs, `.env` rendered from the templates
with host secrets preserved, systemd units, and the Caddy vhost — then reloads
Caddy. A fresh box, a new site, a changed unit, or a changed Caddyfile are all
handled. You do **not** need to run the manual runbook below for a normal cutover;
it's kept as the explanatory reference / break-glass procedure.

The only non-automated prerequisites: (a) the five **NEW DNS records** (grey-cloud,
above), and (b) on a never-before-used box, `python3 -m venv` must work
(`sudo apt-get install -y python3.11-venv` once — see Gotchas).

## CUTOVER RUNBOOK (manual reference / break-glass)

No users yet, so downtime is irrelevant — this can be a clean, deliberate
cutover. These are the steps `remote-update.sh` now performs automatically; run
them by hand only to debug or to provision without CI. Order is
DNS → code → provision → units → Caddy → restart → smoke.

1. **Get the new repo state onto the box.** Either let the Deploy workflow rsync
   it (push to `main`), or rsync manually (see *Manual sync* below). The rsync
   protects `.env` / `.venv` / `data` via excludes.

2. **DNS.** Add the five NEW grey-cloud A records above. Wait for them to resolve.

3. **Provision the NEW sites.** First the content sites (no secrets):
   ```bash
   for s in wiki learn project; do
     cd /opt/axenstax/tools/sites/$s
     python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
     cp /opt/axenstax/tools/sites/deploy/env.$s.template .env
   done
   ```
   Then **claim** — it needs a Printful token + claim codes (see
   `tools/sites/claim/README.md`):
   ```bash
   cd /opt/axenstax/tools/sites/claim
   python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
   cp /opt/axenstax/tools/sites/deploy/env.claim.template .env
   # edit .env: set PRINTFUL_TOKEN=... (keep PRINTFUL_AUTO_CONFIRM=false to start)
   .venv/bin/python gen_codes.py 50 --csv > prague-codes.csv   # generate your codes
   ```

4. **Refresh the 3 EXISTING sites' `.env`** from the new templates — they now
   carry the new cross-site URLs. **`game/.env` holds secrets** (`NOSTR_SERVER_KEY`,
   maybe `BLOSSOM_PUBLIC_URL`) — do NOT blindly overwrite it; update only the
   `*_URL` / `CORS_ORIGINS` lines, or re-copy the template and re-add the secret:
   ```bash
   cp /opt/axenstax/tools/sites/deploy/env.docs.template      /opt/axenstax/tools/sites/docs/.env
   cp /opt/axenstax/tools/sites/deploy/env.marketing.template /opt/axenstax/tools/sites/marketing/.env
   # game: edit GAME_URL/DOCS_URL/MARKETING_URL/CORS_ORIGINS in place,
   #       preserving NOSTR_SERVER_KEY and any BLOSSOM_PUBLIC_URL.
   ```

5. **Install the 3 NEW systemd units + enable:**
   ```bash
   sudo cp /opt/axenstax/tools/sites/deploy/axenstax-{wiki,learn,project,claim}.service /etc/systemd/system/
   sudo systemctl daemon-reload
   sudo systemctl enable --now axenstax-wiki axenstax-learn axenstax-project axenstax-claim
   ```

6. **Swap the Caddy vhost** (routes the new host map; issues certs on reload):
   ```bash
   sudo cp -a /etc/caddy/Caddyfile /etc/caddy/Caddyfile.bak.$(date +%Y%m%d-%H%M%S)
   sudo cp /opt/axenstax/tools/sites/deploy/axenstax.Caddyfile /etc/caddy/conf.d/
   sudo caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
   sudo systemctl reload caddy
   ```

7. **Restart the 3 existing services** (they have new `.env`):
   ```bash
   sudo systemctl restart axenstax-game axenstax-docs axenstax-marketing
   ```

8. **Smoke test** all eight:
   ```bash
   for u in https://axenstax.com/ https://play.axenstax.com/ \
            https://play.axenstax.com/game https://learn.axenstax.com/ \
            https://wiki.axenstax.com/ https://claim.axenstax.com/ \
            https://axenstax.org/ https://docs.axenstax.org/; do
     echo "$u -> $(curl -s -o /dev/null -w '%{http_code}' -L --max-time 25 "$u")"
   done
   ```

After step 6 the new hosts get Let's Encrypt certs automatically (grey-cloud DNS
must already resolve). `SOURCE_URL` is set to the public repo in
`env.marketing.template` and `env.project.template`; `render-env.py` treats a
non-blank template value as authoritative, so it reaches production on the next deploy.

### Manual sync (instead of the workflow)

```bash
KEY=~/.ssh/<deploy-key>
ssh -i "$KEY" deploy@DEPLOY_HOST 'mkdir -p /opt/axenstax/tools/sites /opt/axenstax/game/engine/dist'
rsync -az --delete --exclude .venv --exclude .env --exclude certs --exclude data \
  --exclude __pycache__ -e "ssh -i $KEY" \
  /path/to/AxeNStax/tools/sites/ deploy@DEPLOY_HOST:/opt/axenstax/tools/sites/
rsync -az --delete -e "ssh -i $KEY" \
  /path/to/AxeNStax/game/engine/dist/ deploy@DEPLOY_HOST:/opt/axenstax/game/engine/dist/
```

## CI/CD (auto-deploy)

`.github/workflows/deploy.yml` auto-deploys on every push to `main`. It builds the
WASM bundle, rsyncs sites + bundle, runs `remote-update.sh`, then smoke-tests the
public URLs.

- `remote-update.sh` is **self-provisioning and idempotent**: for each site it
  creates the venv if missing, renders `.env` (via `render-env.py` — template
  config wins, host secrets preserved, `NOSTR_SERVER_KEY` auto-generated),
  installs/refreshes the systemd unit, and restarts. Then it installs/refreshes
  the Caddy vhost and reloads Caddy **only when the file changed**. A new site, a
  changed `*.service`, or a changed `axenstax.Caddyfile` are all applied with no
  manual step.
- The smoke test hits the **new** host map and **retries** each URL (up to ~80s)
  so the first cutover survives Let's Encrypt cert issuance on the new hosts.
- **Prerequisites it does NOT do:** Cloudflare DNS records (add the five new
  grey-cloud A records once, above), and OS packages such as `python3.11-venv` on
  a fresh box.
- **Multi-tenant safety:** only ever touches `/opt/axenstax`, the `axenstax-*`
  units, and `conf.d/axenstax.Caddyfile`. Caddy changes are validated against the
  full config and rolled back on failure, so a bad vhost can't break another tenant.

## If the deploy fails at "Configure SSH"

Both `deploy.yml` and `publish-installers.yml` seed `known_hosts` via
`tools/sites/deploy/ci-ssh-keyscan.sh DEPLOY_HOST` instead of a bare
`ssh-keyscan`. The box's sshd occasionally accepts the TCP connection but
sits silent for a few seconds (HTTPS on the same box stays fine throughout),
and a bare `ssh-keyscan` gives up after its 5s default with no retry and a
swallowed stderr — the Actions log shows nothing but "Process completed with
exit code 1". The script retries (6 attempts, 20s apart, `-T 10` per
attempt) and, on failure, prints a diagnosis distinguishing "TCP
refused/unreachable" (box/network down — check the Hetzner console) from
"TCP accepted but no SSH banner" (sshd itself isn't answering — check `ssh`
to the box directly, or its console, then restart sshd if needed). It also
prints the exact `gh run rerun <run id> --failed` to use once the box is
answering again. Attempts/delay are overridable via `KEYSCAN_ATTEMPTS` /
`KEYSCAN_DELAY` env vars for local testing.

## Gotchas (these cost real time)

- **Never `chmod 600 .../*/.*env`** — the `.*env` glob also matches `.venv/`
  dirs and strips their `+x`, giving `203/EXEC Permission denied`. chmod each
  `.env` by explicit path.
- Box needed `sudo apt-get install -y python3.11-venv` once (ensurepip missing).
- `/opt` is exec-OK (not noexec). ufw opens only 22/80/443, so loopback binds
  are safe; Caddy is the only public entry.
- Cloudflare records must be grey-cloud (DNS-only) or ACME issuance fails.
- The live dev GROQ key lives in `tools/sites/game/.env` — the rsync excludes
  `.env`; never copy it to the host. Prod `.env` is built from the template.
- `game/.env` carries `NOSTR_SERVER_KEY` — don't lose it when refreshing the
  game env in step 4.

## Initial provisioning (fresh box only)

If the box has never hosted these sites, do `venv + pip + .env` for **all six**
sites, install **all six** units, then the Caddy + DNS steps:
```bash
for s in marketing game learn wiki claim project docs; do
  cd /opt/axenstax/tools/sites/$s
  python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
done
# write each .env from env.<site>.template; game: NOSTR_SERVER_KEY=$(openssl rand -hex 32);
# claim: PRINTFUL_TOKEN=... then `gen_codes.py 50 --csv` (see claim/README.md)
sudo cp /opt/axenstax/tools/sites/deploy/axenstax-*.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now axenstax-{marketing,game,learn,wiki,claim,project,docs}
# then the Caddy + DNS steps above.
```
