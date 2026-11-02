#!/usr/bin/env bash
# Does SIGTERM (what `docker stop` sends) reach the dedicated server's final save?
#
# Runs the real engine binary as a dedicated server on a temp worlds dir and a free
# port, lets it create its world, sends a real SIGTERM, and checks that it exited
# cleanly and wrote the world again. Autosave is pushed out of the way, so the only
# save after the first one at boot is the final one. Linux only (reads /proc).
#
# Usage: tools/dedicated-server/test-sigterm-save.sh [--no-cleanup] [engine-binary]
#   --no-cleanup   keep the temp dir and its log for a look afterwards
# The binary is, in order: the argument, $AXENSTAX_ENGINE_BIN, or the debug build in
# $CARGO_TARGET_DIR or the main checkout's game/engine/build. Nothing is built here:
# run `cargo build` in game/engine first. Exit 0 = the final save happened.
set -uo pipefail

cleanup=1
bin="${AXENSTAX_ENGINE_BIN:-}"
for arg in "$@"; do
    case "$arg" in
        --no-cleanup) cleanup=0 ;;
        -*) echo "unknown option: $arg" >&2; exit 2 ;;
        *) bin="$arg" ;;
    esac
done

if [ -z "$bin" ]; then
    here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    common="$(git -C "$here" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
    for candidate in \
        "${CARGO_TARGET_DIR:-/nonexistent}/debug/axenstax-engine" \
        "${common%/.git}/game/engine/build/debug/axenstax-engine"; do
        if [ -x "$candidate" ]; then bin="$candidate"; break; fi
    done
fi
if [ -z "$bin" ] || [ ! -x "$bin" ]; then
    echo "no engine binary found: run 'cargo build' in game/engine, or pass its path" >&2
    exit 2
fi

BOOT_SECS=300
STOP_SECS=120
WORLD=sigterm-test

tmp="$(mktemp -d)"
log="$tmp/server.log"
pid=""
finish() {
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then kill -9 "$pid" 2>/dev/null; fi
    if [ "$cleanup" = 1 ]; then rm -rf -- "$tmp"; else echo "kept: $tmp"; fi
}
trap finish EXIT

port="$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
echo "binary: $bin"
echo "port:   $port"

env -u AXENSTAX_PUBLIC_HOST -u AXENSTAX_DOMAIN -u AXENSTAX_WHITELIST -u AXENSTAX_ALLOW_GUESTS \
    AXENSTAX_DATA_DIR="$tmp/data" AXENSTAX_WORLDS_DIR="$tmp/worlds" RUST_LOG=info \
    "$bin" --server --world "$WORLD" --port "$port" --public-host localhost --autosave 86400 \
    >"$log" 2>&1 &
pid=$!

# Ready when the shutdown handler is installed, which is just before the tick loop.
# SigCgt is a hex mask of caught signals; a handler for SIGINT (2) is bit 1 on every
# build, with or without the fix being tested.
ready=0
for _ in $(seq 1 $((BOOT_SECS * 5))); do
    kill -0 "$pid" 2>/dev/null || break
    mask="$(awk '/^SigCgt:/ { print $2 }' "/proc/$pid/status" 2>/dev/null)"
    if [ -n "$mask" ] && [ $(( 0x$mask & 0x2 )) -ne 0 ]; then ready=1; break; fi
    sleep 0.2
done
if [ "$ready" != 1 ]; then
    echo "FAIL: the server didn't come up within ${BOOT_SECS}s"
    tail -n 20 "$log"
    exit 1
fi

saved="$tmp/worlds/$WORLD/world.dat"
if [ ! -f "$saved" ]; then
    echo "FAIL: no $WORLD/world.dat after boot"
    tail -n 20 "$log"
    exit 1
fi
before="$(stat -c %.9Y "$saved")"
sleep 1

echo "sending SIGTERM to $pid"
kill -TERM "$pid"
stopped=0
for _ in $(seq 1 $((STOP_SECS * 5))); do
    kill -0 "$pid" 2>/dev/null || { stopped=1; break; }
    sleep 0.2
done
if [ "$stopped" != 1 ]; then
    echo "FAIL: still running ${STOP_SECS}s after SIGTERM"
    exit 1
fi
wait "$pid"
status=$?
pid=""
after="$(stat -c %.9Y "$saved")"

fails=0
check() {
    if [ "$2" = yes ]; then echo "  ok    $1"; else echo "  FAIL  $1"; fails=$((fails + 1)); fi
}
answer() { if "$@" >/dev/null 2>&1; then echo yes; else echo no; fi; }
newer() { python3 -c 'import sys; sys.exit(0 if float(sys.argv[1]) > float(sys.argv[2]) else 1)' "$1" "$2"; }

check "exit status 0 (got $status)" "$([ "$status" = 0 ] && echo yes || echo no)"
check "the handler ran" "$(answer grep -q 'Shutdown signal received' "$log")"
check "the final save finished" "$(answer grep -q 'Saved. Goodbye.' "$log")"
check "world.dat was written after the signal ($before -> $after)" "$(answer newer "$after" "$before")"

if [ "$fails" != 0 ]; then
    echo "FAIL: SIGTERM did not reach the final save"
    tail -n 5 "$log"
    exit 1
fi
echo "PASS: SIGTERM reached the final save"
