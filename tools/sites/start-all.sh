#!/bin/bash
# Boot all Axe'n'Stax sites in the background.
# Logs land in /tmp/axenstax-{game,docs,marketing,wiki,learn,project,claim}.log.
# Use `tools/sites/stop-all.sh` to bring them down.
#
# Audience split: .com = product (marketing/play/learn/wiki/claim), .org = project
# (project home / docs). See docs/architecture/2026-06-06-domain-and-site-architecture.md.

set -euo pipefail
cd "$(dirname "$0")"

# Ports — override via env if you need to.
GAME_PORT="${GAME_PORT:-8094}"        # play.axenstax.com
DOCS_PORT="${DOCS_PORT:-8095}"        # docs.axenstax.org
MARKETING_PORT="${MARKETING_PORT:-8096}"  # axenstax.com
WIKI_PORT="${WIKI_PORT:-8097}"        # wiki.axenstax.com
LEARN_PORT="${LEARN_PORT:-8098}"      # learn.axenstax.com
PROJECT_PORT="${PROJECT_PORT:-8099}"  # axenstax.org
CLAIM_PORT="${CLAIM_PORT:-8100}"      # claim.axenstax.com

# Inter-site URLs so each app's templates can link out correctly.
GAME_URL="${GAME_URL:-https://localhost:${GAME_PORT}}"
DOCS_URL="${DOCS_URL:-https://localhost:${DOCS_PORT}}"
MARKETING_URL="${MARKETING_URL:-https://localhost:${MARKETING_PORT}}"
WIKI_URL="${WIKI_URL:-https://localhost:${WIKI_PORT}}"
LEARN_URL="${LEARN_URL:-https://localhost:${LEARN_PORT}}"
PROJECT_URL="${PROJECT_URL:-https://localhost:${PROJECT_PORT}}"

start_one() {
    local name="$1"
    local dir="$2"
    local port_var="$3"
    local log="/tmp/axenstax-${name}.log"
    echo "Starting ${name} (port ${!port_var}) — log: ${log}"
    (
        cd "$dir"
        export GAME_URL DOCS_URL MARKETING_URL WIKI_URL LEARN_URL PROJECT_URL
        export PORT="${!port_var}"
        bash start.sh > "$log" 2>&1
    ) &
    echo "  pid=$!"
}

start_one marketing   ./marketing   MARKETING_PORT
start_one game        ./game        GAME_PORT
start_one learn       ./learn       LEARN_PORT
start_one wiki        ./wiki        WIKI_PORT
start_one claim       ./claim       CLAIM_PORT
start_one project     ./project     PROJECT_PORT
start_one docs        ./docs        DOCS_PORT

echo
echo "All sites starting. Wait a few seconds, then:"
echo "  Marketing (.com)  : ${MARKETING_URL}"
echo "  Play              : ${GAME_URL}"
echo "  Learn             : ${LEARN_URL}"
echo "  Wiki              : ${WIKI_URL}"
echo "  Claim             : https://localhost:${CLAIM_PORT}"
echo "  Project (.org)    : ${PROJECT_URL}"
echo "  Docs              : ${DOCS_URL}"
echo
echo "Tail logs with:  tail -f /tmp/axenstax-{marketing,game,learn,wiki,claim,project,docs}.log"
