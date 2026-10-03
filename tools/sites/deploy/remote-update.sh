#!/usr/bin/env bash
# Post-deploy hook — runs ON the routing box (invoked over SSH by the Deploy
# workflow after rsync). SELF-PROVISIONING + idempotent: stands up the full site
# layout from scratch and keeps it in sync on every push. Safe to run repeatedly.
#
# Per site (.com group: marketing/game/learn/wiki/claim — .org group: project/docs):
#   1. venv          — create if missing, then install requirements.txt
#   2. .env          — render from env.<site>.template merged with any existing
#                      host .env (see render-env.py: template wins for config,
#                      blanks never clobber host secrets, NOSTR_SERVER_KEY auto-gen)
#   3. systemd unit  — install/refresh from deploy/*.service if missing or changed
#   4. restart       — enable + restart, report active/FAILED
# Then once, after the per-site loop:
#   5. Caddy vhost   — install/refresh axenstax.Caddyfile, validate, reload if changed
#
# The .env merge is why this can do the one-time .com/.org cutover (existing
# sites' URLs refresh) without losing GROQ/NOSTR_SERVER_KEY/PRINTFUL_TOKEN.
#
# Multi-tenant box — additive only: touches ONLY /opt/axenstax, the
# axenstax-*.service units, and conf.d/axenstax.Caddyfile. Never another tenant.
set -uo pipefail

DEPLOY_DIR="/opt/axenstax/tools/sites/deploy"
RENDER_ENV="$DEPLOY_DIR/render-env.py"
SITES="marketing game learn wiki claim project docs"
rc=0
units_changed=0

# ---- 1-3: provision venv + .env + unit for each site ----------------------
for s in $SITES; do
  dir="/opt/axenstax/tools/sites/$s"
  unit="axenstax-$s"
  svc="$DEPLOY_DIR/$unit.service"
  tmpl="$DEPLOY_DIR/env.$s.template"

  if [ ! -d "$dir" ]; then
    echo "$unit: no site dir ($dir) — skipping"
    continue
  fi

  # 1. venv
  if [ ! -x "$dir/.venv/bin/pip" ]; then
    echo "$unit: creating venv"
    if ! python3 -m venv "$dir/.venv"; then
      echo "$unit: venv creation FAILED — skipping"
      rc=1; continue
    fi
  fi

  # 2. .env (merge template with existing host .env; preserves secrets)
  if [ -f "$tmpl" ]; then
    if python3 "$RENDER_ENV" "$tmpl" "$dir/.env" > "$dir/.env.tmp" 2>/dev/null; then
      mv "$dir/.env.tmp" "$dir/.env"
      chmod 600 "$dir/.env"   # explicit full path — never a .*env glob (see gotchas)
    else
      rm -f "$dir/.env.tmp"
      echo "$unit: .env render FAILED"
      rc=1
    fi
  elif [ ! -f "$dir/.env" ]; then
    echo "$unit: no template ($tmpl) and no .env — skipping"
    rc=1; continue
  fi

  # 3. deps
  if ! ( cd "$dir" && ./.venv/bin/pip install -q -r requirements.txt ); then
    echo "$unit: pip install FAILED"
    rc=1; continue
  fi

  # 4. systemd unit — install/refresh if missing or changed
  if [ -f "$svc" ] && ! sudo cmp -s "$svc" "/etc/systemd/system/$unit.service"; then
    echo "$unit: installing/refreshing systemd unit"
    sudo cp "$svc" "/etc/systemd/system/$unit.service"
    units_changed=1
  fi
done

[ "$units_changed" = 1 ] && sudo systemctl daemon-reload

# ---- 4: enable + restart each provisioned site ----------------------------
for s in $SITES; do
  dir="/opt/axenstax/tools/sites/$s"
  unit="axenstax-$s"
  [ -x "$dir/.venv/bin/pip" ] || continue
  if ! systemctl list-unit-files "$unit.service" >/dev/null 2>&1; then
    echo "$unit: no systemd unit installed — skipping restart"
    continue
  fi
  sudo systemctl enable "$unit" >/dev/null 2>&1 || true
  sudo systemctl restart "$unit"
  sleep 1
  if systemctl is-active --quiet "$unit"; then
    echo "$unit: active"
  else
    echo "$unit: FAILED"
    sudo journalctl -u "$unit" -n 20 --no-pager || true
    rc=1
  fi
done

# ---- 5: Caddy vhost — install/refresh + reload if changed -----------------
CADDY_SRC="$DEPLOY_DIR/axenstax.Caddyfile"
CADDY_DST="/etc/caddy/conf.d/axenstax.Caddyfile"
if [ -f "$CADDY_SRC" ]; then
  if sudo cmp -s "$CADDY_SRC" "$CADDY_DST"; then
    echo "caddy: vhost unchanged"
  else
    echo "caddy: vhost changed — installing + validating"
    sudo mkdir -p /etc/caddy/conf.d
    backup=""
    if [ -f "$CADDY_DST" ]; then
      backup="$CADDY_DST.bak.$(date +%Y%m%d-%H%M%S)"
      sudo cp -a "$CADDY_DST" "$backup"
    fi
    sudo cp "$CADDY_SRC" "$CADDY_DST"
    if sudo caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile; then
      sudo systemctl reload caddy && echo "caddy: reloaded (certs auto-issue on first serve)"
    else
      echo "caddy: validate FAILED — rolling back"
      if [ -n "$backup" ]; then
        sudo cp "$backup" "$CADDY_DST"
      else
        sudo rm -f "$CADDY_DST"   # we created it this run; remove so config stays valid
      fi
      rc=1
    fi
  fi
fi

exit $rc
