#!/usr/bin/env bash
# Build the Axe'n'Stax dedicated-server image.
#
# Compiles the release `--server` binary and the guest-boot web bundle ON THE
# HOST (per CLAUDE.md: builds run on the host, not in the container), stages them
# under ./stage/, then builds the Docker image. Re-run after engine changes.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
ENGINE="$ROOT/game/engine"
STAGE="$HERE/stage"

echo "==> [1/4] building release server binary"
( cd "$ENGINE" && CARGO_TARGET_DIR="$ROOT/build" cargo build --release --bin axenstax-engine )

echo "==> [2/4] building guest-boot web bundle (index.dedicated.html)"
# NB: the index file must be the positional TARGET *before* --release; trunk's
# `--release [<RELEASE>]` takes an optional value and would otherwise swallow it.
( cd "$ENGINE" && trunk build index.dedicated.html --release )

echo "==> [3/4] staging artifacts"
rm -rf "$STAGE"
mkdir -p "$STAGE/web"
cp "$ROOT/build/release/axenstax-engine" "$STAGE/axenstax-engine"
cp -r "$ENGINE/dist/." "$STAGE/web/"

echo "==> [4/4] building Docker images (server + console), tagged as the GHCR names"
( cd "$HERE" && docker compose -f docker-compose.build.yml build )

cat <<'EOF'

Done. The images are tagged ghcr.io/decented/axenstax-{server,operator-console}:latest,
so the pull file finds them locally — start the server with:

    cd tools/dedicated-server && docker compose up -d

Then share, from any machine on the same network:

    Web    →  https://<this-box-ip>:8443   (accept the cert warning once)
    Native →  Join → ws://<this-box-ip>:6767

EOF
