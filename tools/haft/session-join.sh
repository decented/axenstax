#!/usr/bin/env bash
# Put Haft in a room and let a Claude Code session be his brain.
#
# WHY THIS RATHER THAN AN API KEY
#
#   The other route (tools/haft/haft.service + ANTHROPIC_API_KEY in haft.env)
#   runs Haft as an always-on daemon on its own model key. That is right for a
#   standing room nobody is watching.
#
#   This route is the owner's preference and is different in kind: no API key
#   exists anywhere, and Haft's replies come from whatever Claude session is
#   driving him. Hand a session the room link, it runs this, and it speaks and
#   listens through the pipes below. Nothing is billed to a key, nothing runs
#   when nobody is home, and the model in the room is the same one you are
#   already talking to.
#
#   The trade is honest and worth stating: Haft answers at the speed the session
#   notices, not instantly, and he goes silent the moment the session ends. A
#   room that needs a reply at 3am wants the daemon; a room you are sitting in
#   wants this.
#
# USAGE
#
#   tools/haft/session-join.sh '<room link>'
#
#   Then, from the driving session:
#     say :   printf '%s\n' '{"op":"say","text":"..."}' > "$HAFT_IN"
#     listen: tail -n 40 "$HAFT_OUT"
#     roster: printf '%s\n' '{"op":"roster"}' > "$HAFT_IN"
#     leave : printf '%s\n' '{"op":"leave"}' > "$HAFT_IN"
#
#   The seam is newline-delimited JSON in both directions — one object per
#   line. A newline inside a message would split one command into two, so let
#   a JSON encoder do the escaping rather than hand-building the string.

set -euo pipefail

LINK="${1:-}"
if [ -z "$LINK" ]; then
  echo "usage: $0 '<room link>'" >&2
  echo >&2
  echo "The link is printed by tools/room/create-room.mjs and written beside" >&2
  echo "its state file as <state>.link." >&2
  exit 2
fi

KITHMOOT_DIR="${KITHMOOT_DIR:-$HOME/kithmoot}"
IDENTITY="${HAFT_IDENTITY:-$HOME/.kithmoot/haft.key}"
PERSONA="${HAFT_PERSONA:-$(cd "$(dirname "$0")" && pwd)/haft-persona.md}"
PROOF="${HAFT_OWNER_PROOF:-$HOME/.config/axenstax/haft-owner-proof.json}"
RUN="${XDG_RUNTIME_DIR:-/tmp}/haft"

if [ ! -f "$KITHMOOT_DIR/bin/kithmoot-agent.mjs" ]; then
  echo "No kithmoot checkout at $KITHMOOT_DIR — set KITHMOOT_DIR." >&2
  exit 1
fi

# Refuse rather than let the CLI mint a fresh key: an agent running as the wrong
# key looks exactly like one running correctly, and its ownership proof would
# name a stranger.
if [ ! -f "$IDENTITY" ]; then
  echo "No identity at $IDENTITY. Refusing to start — a fresh key would not" >&2
  echo "match Haft's ownership proof, and that failure is silent." >&2
  exit 1
fi

if [ ! -f "$PROOF" ]; then
  echo "No ownership proof at $PROOF." >&2
  echo "A room requiring owned agents will refuse Haft without it." >&2
  echo "See tools/haft/README.md for how the owner mints one." >&2
  exit 1
fi

mkdir -p "$RUN"
IN="$RUN/in"
OUT="$RUN/out"
ERR="$RUN/err"

# A fresh FIFO each start: a stale one may still have a writer attached from a
# previous run, and Haft would then read somebody else's commands.
rm -f "$IN"
mkfifo "$IN"
: > "$OUT"
: > "$ERR"

cd "$KITHMOOT_DIR"

# Hold the FIFO open from inside the wrapper. Without a persistent writer the
# agent sees EOF the moment the first `printf >` finishes and treats it as
# stdin closing.
setsid bash -c "exec 3<>'$IN'; node bin/kithmoot-agent.mjs join '$LINK' \
  --name 'Haft McHafty' --identity '$IDENTITY' --brain stdio \
  --owner-proof '$PROOF' --persona '$PERSONA' \
  < '$IN' > '$OUT' 2> '$ERR'" </dev/null >/dev/null 2>&1 &
disown

echo "Haft starting."
echo
echo "  HAFT_IN=$IN"
echo "  HAFT_OUT=$OUT"
echo "  HAFT_ERR=$ERR"
echo
echo "say:    printf '%s\\n' '{\"op\":\"say\",\"text\":\"hello\"}' > $IN"
echo "listen: tail -n 40 $OUT"
echo
echo "Watch $OUT for a {\"type\":\"ready\"} line — that means he is in."
echo "If it never appears, read $ERR."
