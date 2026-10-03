#!/usr/bin/env bash
# Start Caddy + the Axe'n'Stax game server, and forward SIGTERM so the world saves
# cleanly on `docker stop`.
#
# New in the setup-wizard build (docs/superpowers/specs/2026-06-21-server-setup-wizard-design.md):
#   • First-run GATE — Caddy (hence the Operator Console at /admin) comes up
#     immediately, but the engine WAITS until the operator finishes the setup
#     wizard (the console writes .identity/setup-complete). That way the operator's
#     chosen game mode + kind-of-place apply to the FIRST world, with no throwaway
#     default world. Set AXENSTAX_SKIP_WIZARD=1 to boot immediately on the compose
#     defaults (no gate).
#   • CONFIG SOURCE — before launching, the engine sources .identity/server.env
#     (shell-quoted AXENSTAX_* overrides the console wrote); these win over the
#     compose defaults.
#   • RESTART-WATCHER — .identity/restart cycles the engine (re-sourcing the env);
#     .identity/reset-world archives the world so it's recreated fresh in a newly
#     chosen mode. Both are honoured here because the console runs in a SEPARATE
#     container and can't `docker restart` the engine itself.
#
# NB: not `set -e` — the supervise loop handles failures itself.
set -uo pipefail

WORLDS="${AXENSTAX_WORLDS_DIR:-/worlds}"
IDDIR="$WORLDS/.identity"
mkdir -p "$WORLDS" /certs "$IDDIR"

# Pick the TLS front. With AXENSTAX_DOMAIN set (a VPS with a real DNS name),
# use the ACME Caddyfile — Caddy fetches a real Let's Encrypt cert, no warning.
# Otherwise fall back to the self-signed :8443 front (LAN / no-domain).
if [ -n "${AXENSTAX_DOMAIN:-}" ]; then
    CADDYFILE=/etc/caddy/Caddyfile.domain
    echo "[entrypoint] ACME mode — serving https://${AXENSTAX_DOMAIN} (needs :80 + :443 reachable)"
else
    CADDYFILE=/etc/caddy/Caddyfile
    if [ ! -s /certs/server.crt ] || [ ! -s /certs/server.key ]; then
        echo "[entrypoint] generating self-signed TLS cert…"
        openssl req -x509 -newkey rsa:2048 -nodes \
            -keyout /certs/server.key -out /certs/server.crt -days 3650 \
            -subj "/CN=axenstax-server" \
            -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" >/dev/null 2>&1
    fi
fi

SERVER_PID=""
CADDY_PID=""

shutdown() {
    echo "[entrypoint] shutting down…"
    [ -n "$SERVER_PID" ] && kill -TERM "$SERVER_PID" 2>/dev/null || true
    [ -n "$SERVER_PID" ] && wait "$SERVER_PID" 2>/dev/null || true
    [ -n "$CADDY_PID" ] && kill -TERM "$CADDY_PID" 2>/dev/null || true
    exit 0
}
trap shutdown TERM INT

# Caddy front (HTTPS + static + /admin console + ws proxy). Started FIRST so the
# Operator Console is reachable while the engine waits for first-run setup.
caddy run --config "$CADDYFILE" --adapter caddyfile &
CADDY_PID=$!

# ── first-run gate ──────────────────────────────────────────────────────────────
# The engine doesn't create/serve a world until the operator finishes setup (the
# console writes .identity/setup-complete). Skip with AXENSTAX_SKIP_WIZARD=1.
if [ "${AXENSTAX_SKIP_WIZARD:-0}" != "1" ]; then
    announced=0
    while [ ! -f "$IDDIR/setup-complete" ]; do
        if ! kill -0 "$CADDY_PID" 2>/dev/null; then
            echo "[entrypoint] caddy exited while waiting for setup — aborting"
            exit 1
        fi
        if [ "$announced" = 0 ]; then
            echo "[entrypoint] waiting for first-run setup — open /admin and finish the wizard…"
            announced=1
        fi
        sleep 3
    done
fi

# ── launch the engine (sourcing the wizard's chosen config) ─────────────────────
launch_engine() {
    # Honour a one-shot fresh-world request: the operator chose a new kind of place
    # that needs a brand-new world (game mode is baked in at world creation). The
    # current world is ARCHIVED (renamed aside), never deleted — recoverable.
    if [ -f "$IDDIR/reset-world" ]; then
        rm -f "$IDDIR/reset-world"
        wdir="$WORLDS/${AXENSTAX_WORLD:-server-world}"
        if [ -d "$wdir" ]; then
            ts="$(date +%Y%m%d-%H%M%S)"
            echo "[entrypoint] archiving world → ${wdir}.archived-${ts}"
            mv "$wdir" "${wdir}.archived-${ts}" 2>/dev/null || true
        fi
    fi
    # The console writes shell-quoted AXENSTAX_* overrides here; sourced AFTER the
    # base (compose) env is in place, so the wizard's choices win.
    if [ -f "$IDDIR/server.env" ]; then
        # shellcheck disable=SC1090
        set -a; . "$IDDIR/server.env"; set +a
    fi
    rm -f "$IDDIR/restart"
    echo "[entrypoint] starting game server (gamemode=${AXENSTAX_GAMEMODE:-survival}, showcase=${AXENSTAX_SHOWCASE:-0})"
    axenstax-engine --server &
    SERVER_PID=$!
}
launch_engine

# ── supervise: honour restart requests; exit if caddy/engine die otherwise ──────
while true; do
    sleep 2
    # Operator asked for a clean restart (new config / fresh world).
    if [ -f "$IDDIR/restart" ]; then
        echo "[entrypoint] restart requested — cycling the game server"
        kill -TERM "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
        launch_engine
        continue
    fi
    # Caddy died → the box is unusable; let the container restart.
    if ! kill -0 "$CADDY_PID" 2>/dev/null; then
        echo "[entrypoint] caddy exited — shutting down"
        shutdown
    fi
    # Engine exited on its own (crash / clean exit; save already flushed) → restart container.
    if [ -n "$SERVER_PID" ] && ! kill -0 "$SERVER_PID" 2>/dev/null; then
        echo "[entrypoint] game server exited — shutting down"
        shutdown
    fi
done
