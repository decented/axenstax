#!/usr/bin/env bash
# Does SIGTERM (what `docker stop` sends) reach the dedicated server's final save?
#
# Runs the real engine binary as a dedicated server on a temp worlds dir and a free
# port, lets it create its world, sends a real SIGTERM, and checks that it exited
# cleanly and wrote the world again. Autosave is pushed out of the way, so the only
# save after the first one at boot is the final one. Linux only (reads /proc).
#
# It runs twice: once normally, and once with SIGHUP ignored, the way `nohup` starts
# a server. The second run checks that the server keeps ignoring SIGHUP (it survives
# one) and still saves on SIGTERM.
#
# Usage: tools/dedicated-server/test-sigterm-save.sh [--no-cleanup] <engine-binary>
#   --no-cleanup   keep the temp dirs and their logs for a look afterwards
# The binary is required: say which build you mean, so a stale build from another
# tree can't pass for the one under test. Nothing is built here: run `cargo build` in
# game/engine first. Exit 0 = the final save happened in both runs.
set -uo pipefail

cleanup=1
bin=""
for arg in "$@"; do
    case "$arg" in
        --no-cleanup) cleanup=0 ;;
        -*) echo "unknown option: $arg" >&2; exit 2 ;;
        *) bin="$arg" ;;
    esac
done

if [ -z "$bin" ] || [ ! -x "$bin" ]; then
    echo "usage: $0 [--no-cleanup] <engine-binary>   (an executable path is required)" >&2
    exit 2
fi

BOOT_SECS=300
STOP_SECS=120
WORLD=sigterm-test

pid=""
tmps=()
finish() {
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then kill -9 "$pid" 2>/dev/null; fi
    for t in "${tmps[@]}"; do
        if [ "$cleanup" = 1 ]; then rm -rf -- "$t"; else echo "kept: $t"; fi
    done
}
trap finish EXIT

fails=0
check() {
    if [ "$2" = yes ]; then echo "  ok    $1"; else echo "  FAIL  $1"; fails=$((fails + 1)); fi
}
answer() { if "$@" >/dev/null 2>&1; then echo yes; else echo no; fi; }
newer() { python3 -c 'import sys; sys.exit(0 if float(sys.argv[1]) > float(sys.argv[2]) else 1)' "$1" "$2"; }
# Is signal number $2 in the hex mask field $1 (SigIgn, SigCgt) of /proc/<pid>/status?
sigbit() {
    local mask
    mask="$(awk -v f="$1:" '$1 == f { print $2 }' "/proc/$pid/status" 2>/dev/null)"
    [ -n "$mask" ] && [ $(( 0x$mask & (1 << ($2 - 1)) )) -ne 0 ]
}

# scenario <label> <hup>   hup = ignore: start with SIGHUP ignored, as nohup does.
scenario() {
    local label="$1" hup="$2" caught tmp log port saved before after status ready
    echo "== $label"
    tmp="$(mktemp -d)" || exit 2
    tmps+=("$tmp")
    log="$tmp/server.log"
    port="$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
    echo "binary: $bin"
    echo "port:   $port"

    # An ignored signal stays ignored across exec, which is how nohup passes it on.
    (
        [ "$hup" = ignore ] && trap '' HUP
        exec env -u AXENSTAX_PUBLIC_HOST -u AXENSTAX_DOMAIN -u AXENSTAX_WHITELIST -u AXENSTAX_ALLOW_GUESTS \
            AXENSTAX_DATA_DIR="$tmp/data" AXENSTAX_WORLDS_DIR="$tmp/worlds" RUST_LOG=info \
            "$bin" --server --world "$WORLD" --port "$port" --public-host localhost --autosave 86400
    ) >"$log" 2>&1 &
    pid=$!

    # Ready when the shutdown handler is installed, which is just before the tick loop.
    # A handler for SIGINT (2) is there on every build, with or without the fix being tested.
    ready=0
    for _ in $(seq 1 $((BOOT_SECS * 5))); do
        kill -0 "$pid" 2>/dev/null || break
        if sigbit SigCgt 2; then ready=1; break; fi
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

    if [ "$hup" = ignore ]; then
        check "SIGHUP is still ignored once the server is up" "$(answer sigbit SigIgn 1)"
        if sigbit SigCgt 1; then caught=no; else caught=yes; fi
        check "and has no handler of ours installed over it" "$caught"
        echo "sending SIGHUP to $pid"
        kill -HUP "$pid"
        sleep 2
        check "the server survived SIGHUP" "$(answer kill -0 "$pid")"
        if ! kill -0 "$pid" 2>/dev/null; then
            wait "$pid"; pid=""
            echo "FAIL: SIGHUP stopped a server that was started with it ignored"
            tail -n 5 "$log"
            exit 1
        fi
    fi

    echo "sending SIGTERM to $pid"
    kill -TERM "$pid"
    local stopped=0
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

    check "exit status 0 (got $status)" "$([ "$status" = 0 ] && echo yes || echo no)"
    check "the handler ran" "$(answer grep -q 'Shutdown signal received' "$log")"
    check "the final save finished" "$(answer grep -q 'Saved. Goodbye.' "$log")"
    check "world.dat was written after the signal ($before -> $after)" "$(answer newer "$after" "$before")"

    if [ "$fails" != 0 ]; then
        echo "FAIL: $label: the final save did not happen"
        tail -n 5 "$log"
        exit 1
    fi
}

scenario "SIGTERM" normal
scenario "SIGTERM after SIGHUP, started with SIGHUP ignored (nohup)" ignore
echo "PASS: SIGTERM reached the final save, with and without SIGHUP ignored"
