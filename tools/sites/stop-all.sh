#!/bin/bash
# Stop all six Axe'n'Stax sites.
# Targets any python/uvicorn process holding ports 8094–8099
# (or override via {GAME,DOCS,MARKETING,WIKI,LEARN,PROJECT}_PORT).

set -uo pipefail

GAME_PORT="${GAME_PORT:-8094}"
DOCS_PORT="${DOCS_PORT:-8095}"
MARKETING_PORT="${MARKETING_PORT:-8096}"
WIKI_PORT="${WIKI_PORT:-8097}"
LEARN_PORT="${LEARN_PORT:-8098}"
PROJECT_PORT="${PROJECT_PORT:-8099}"
CLAIM_PORT="${CLAIM_PORT:-8100}"
SIGNATURE='python.*app\.py|uvicorn'

stop_one() {
    local name="$1"
    local port="$2"
    if ! command -v fuser >/dev/null 2>&1; then
        echo "fuser not installed — can't identify owners; skipping ${name}"
        return
    fi
    pid=$(fuser -n tcp "$port" 2>/dev/null | awk '{print $1}' || true)
    if [ -z "$pid" ]; then
        echo "${name} (:${port}) — not running"
        return
    fi
    cmd=$(ps -p "$pid" -o args= 2>/dev/null || true)
    if echo "$cmd" | grep -Eq "$SIGNATURE"; then
        echo "Stopping ${name} (:${port}) pid=${pid}"
        kill "$pid" 2>/dev/null || true
    else
        echo "${name} (:${port}) — pid ${pid} doesn't look like one of ours: ${cmd}"
    fi
}

stop_one marketing "$MARKETING_PORT"
stop_one game      "$GAME_PORT"
stop_one learn     "$LEARN_PORT"
stop_one wiki      "$WIKI_PORT"
stop_one claim     "$CLAIM_PORT"
stop_one project   "$PROJECT_PORT"
stop_one docs      "$DOCS_PORT"
