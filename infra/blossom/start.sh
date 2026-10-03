#!/usr/bin/env bash
# Axe'n'Stax — Blossom blob store (cloud save bridge tier).
set -euo pipefail
cd "$(dirname "$0")"

# Refuse to launch while the owner pubkey is still the placeholder — otherwise
# the upload allow-list is meaningless and (depending on the image) uploads are
# open to any signer. Fill it in per README "Generating the server key".
if grep -q "REPLACE_WITH_BLOSSOM_APP_PUBKEY_HEX" config.yml; then
  echo "ERROR: config.yml still has the placeholder owner pubkey." >&2
  echo "Generate the server key and fill 'owners:' before starting — see README.md." >&2
  exit 1
fi

docker compose up -d
echo "Blossom up on http://127.0.0.1:3000 (loopback; the game site proxies to it)."
echo "NOTE: config.yml schema is UNVERIFIED against this image — see the header in config.yml."
