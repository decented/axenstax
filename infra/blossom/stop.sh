#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
docker compose down
echo "Blossom stopped (blob volume 'blossom-data' preserved)."
