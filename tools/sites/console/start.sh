#!/usr/bin/env bash
# Axe'n'Stax Operator Console (web sidecar) — local dev launcher.
# Production runs in the dedicated-server Docker compose (see README.md).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

PORT="${PORT:-8101}"

# Refuse to clobber an unrelated process on the port; kill a stale console.
if lsof -ti tcp:"$PORT" >/dev/null 2>&1; then
  pid="$(lsof -ti tcp:"$PORT")"
  if ps -p "$pid" -o args= | grep -q "app.py\|uvicorn"; then
    echo "Stopping stale console on :$PORT (pid $pid)"; kill "$pid" || true; sleep 1
  else
    echo "Port $PORT is held by something else (pid $pid) — aborting." >&2; exit 1
  fi
fi

if [ ! -d .venv ]; then
  echo "Creating venv…"
  python3 -m venv .venv
fi
# shellcheck disable=SC1091
source .venv/bin/activate
pip install -q -r requirements.txt

echo "Operator Console on http://localhost:$PORT  (identity dir: ${AXENSTAX_IDENTITY_DIR:-${AXENSTAX_WORLDS_DIR:-/worlds}/.identity})"
exec python app.py
