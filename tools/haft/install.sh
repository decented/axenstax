#!/usr/bin/env bash
set -euo pipefail

# install.sh - installs Haft McHafty as a user-level systemd unit on THIS
# machine. Run as the owner's own user; no sudo, because there is nothing
# here for root to do — the unit runs under `systemctl --user`, which is
# the whole point (it sleeps when the laptop does).
#
# What this does, in order:
#   1. checks the kithmoot dist/ this unit will run from actually exists
#      (default $HOME/kithmoot; override with KITHMOOT_DIR=/path/to/checkout
#      if yours lives elsewhere — e.g. under whatever workspace directory
#      you keep your other checkouts in)
#   2. refuses if ~/.kithmoot/haft.key is missing, rather than letting the
#      first `systemctl --user start` silently mint Haft a fresh, unrecorded
#      key (see haft.env.example's KITHMOOT_IDENTITY comment)
#   3. renders haft.env.example into ~/.config/axenstax/haft.env, mode
#      0600, filling in $HOME / $AXENSTAX_PERSONA / $AXENSTAX_KITHMOOT_DIR
#      — ONLY if that file does not already exist; an existing env file
#      (with a real room link and owner proof filled in) is never touched
#   4. installs haft.service verbatim to ~/.config/systemd/user/ (it needs
#      no per-machine substitution — see its own header)
#   5. runs `systemctl --user daemon-reload`
#
# What this does NOT do: start the service, enable it, or go looking for
# the room link or the ownership proof. Both of those are Phase 9 work (a
# keeper has to be running, and the owner has to run `kithmoot-agent
# attest`) and neither belongs in an idempotent installer. See
# tools/haft/README.md for the commands left for the owner.
#
# Idempotent: safe to run again after the kithmoot tree is rebuilt, or after
# haft.service changes in this repo — it always reinstalls the unit file
# and reloads, and always leaves the env file and the key alone.

KITHMOOT_DIR="${KITHMOOT_DIR:-$HOME/kithmoot}"
HAFT_KEY="$HOME/.kithmoot/haft.key"
AXENSTAX_CONFIG_DIR="$HOME/.config/axenstax"
ENV_FILE="$AXENSTAX_CONFIG_DIR/haft.env"
SYSTEMD_USER_DIR="$HOME/.config/systemd/user"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"
PERSONA_PATH="$SCRIPT_DIR/haft-persona.md"

echo "==> checking kithmoot is built"
if [[ ! -f "$KITHMOOT_DIR/dist/src/node/cli.js" ]]; then
  echo "install.sh: $KITHMOOT_DIR/dist/src/node/cli.js not found." >&2
  echo "  kithmoot-agent only runs from dist/ (bin/kithmoot-agent.mjs imports" >&2
  echo "  ../dist/src/node/cli.js). Build it there first (npm ci && npm run" >&2
  echo "  build:lib in that checkout) — this installer does not build for" >&2
  echo "  you. If kithmoot lives somewhere other than \$HOME/kithmoot on this" >&2
  echo "  machine, re-run as: KITHMOOT_DIR=/path/to/kithmoot $0" >&2
  exit 1
fi

echo "==> checking Haft's identity key exists"
if [[ ! -f "$HAFT_KEY" ]]; then
  echo "install.sh: $HAFT_KEY does not exist." >&2
  echo "  Refusing to install. kithmoot-agent mints a FRESH key the first" >&2
  echo "  time it runs against a --identity file that isn't there, silently," >&2
  echo "  and an agent running as the wrong key looks exactly like one" >&2
  echo "  running correctly. Create the real key first." >&2
  exit 1
fi
key_mode="$(stat -c '%a' "$HAFT_KEY" 2>/dev/null || stat -f '%Lp' "$HAFT_KEY")"
if [[ "$key_mode" != "600" ]]; then
  echo "install.sh: warning: $HAFT_KEY is not mode 600 (found $key_mode). Fix with: chmod 600 $HAFT_KEY" >&2
fi

echo "==> preparing $AXENSTAX_CONFIG_DIR"
install -d -m 0700 "$AXENSTAX_CONFIG_DIR"

if [[ -s "$ENV_FILE" ]]; then
  echo "==> keeping the existing $ENV_FILE (not overwritten)"
else
  echo "==> writing $ENV_FILE from haft.env.example"
  # Plain text substitution, not shell expansion: haft.env.example's three
  # tokens ($HOME, $AXENSTAX_PERSONA, $AXENSTAX_KITHMOOT_DIR) are literal
  # placeholders, not variables systemd's EnvironmentFile would ever expand
  # itself (it doesn't expand anything — see systemd.exec(5)). This is the
  # one place they become real paths.
  sed \
    -e "s|\$AXENSTAX_KITHMOOT_DIR|$KITHMOOT_DIR|g" \
    -e "s|\$AXENSTAX_PERSONA|$PERSONA_PATH|g" \
    -e "s|\$HOME|$HOME|g" \
    "$SCRIPT_DIR/haft.env.example" > "$ENV_FILE"
  chmod 0600 "$ENV_FILE"
  echo "    edit it: at minimum KITHMOOT_LINK and KITHMOOT_OWNER_PROOF are" >&2
  echo "    still blank (see tools/haft/README.md for how to fill them in)." >&2
fi

echo "==> installing the unit"
install -d -m 0755 "$SYSTEMD_USER_DIR"
install -m 0644 "$SCRIPT_DIR/haft.service" "$SYSTEMD_USER_DIR/haft.service"

echo "==> systemctl --user daemon-reload"
systemctl --user daemon-reload

cat <<NEXT

==> installed, not started.

Haft will not answer in any room until two things exist that this script
does not and cannot create:

  1. A room link in $ENV_FILE (KITHMOOT_LINK=). The room doesn't exist
     until a keeper is running — see
     docs/foundations/2026-09-05-world-chat.md §4 for the KithMootKeeper
     plug, or start one by hand for now.

  2. An ownership proof at the path named by KITHMOOT_OWNER_PROOF in that
     same file. Only the owner can make this — it is signed by the owner's
     own key, never by this session or this script:

       node bin/kithmoot-agent.mjs attest \\
         --agent <Haft's npub, from tools/haft/README.md> \\
         --identity <YOUR OWN key file> \\
         --label "Haft McHafty" \\
         --expires 90d

     Save its stdout to the path in KITHMOOT_OWNER_PROOF.

Once both are in place:

  systemctl --user enable --now haft
  systemctl --user status haft
  journalctl --user -u haft -f

Full walkthrough, including how to read off Haft's own npub for
--expect-pubkey and for the attest command above: tools/haft/README.md
NEXT
