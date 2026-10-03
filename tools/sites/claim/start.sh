#!/bin/bash
# Axe'n'Stax — merch claim / fulfilment intake (port 8100).

set -euo pipefail
cd "$(dirname "$0")"

PORT="${PORT:-8100}"
SIGNATURE='python.*app\.py|uvicorn'

free_port_or_refuse() {
    if ! command -v fuser >/dev/null 2>&1; then return 0; fi
    pid=$(fuser -n tcp "$PORT" 2>/dev/null | awk '{print $1}' || true)
    if [ -z "$pid" ]; then return 0; fi
    cmd=$(ps -p "$pid" -o args= 2>/dev/null || true)
    if echo "$cmd" | grep -Eq "$SIGNATURE"; then
        echo "Killing stale claim site on :$PORT (pid=$pid)"
        kill "$pid" 2>/dev/null || true
        for _ in 1 2 3 4 5; do
            sleep 1
            fuser -n tcp "$PORT" 2>/dev/null >/dev/null || return 0
        done
        kill -9 "$pid" 2>/dev/null || true
        sleep 1
        return 0
    fi
    echo "Refusing to start: port $PORT is held by something else." >&2
    echo "  pid=$pid cmd=$cmd" >&2
    exit 1
}

free_port_or_refuse

if [ ! -d .venv ]; then
    echo "Creating virtual environment..."
    python3 -m venv .venv
    .venv/bin/pip install -r requirements.txt
fi

if [ -f .env ]; then set -a; source .env; set +a; fi

export PORT
exec .venv/bin/python app.py
