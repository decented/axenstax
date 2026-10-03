#!/usr/bin/env bash
# Axe'n'Stax verification harness. One command, green or red.
#
# Default: clippy + build + test + trunk build + forbidden-symbol + bundle-size gates.
# With --smoke: also runs Playwright smoke against a running site.
# With --release: uses --release profile for cargo (slower, matches CI).
#
# The game site must already be running at https://localhost:8094 when --smoke
# is used. Start it with `tools/sites/start-all.sh` in another terminal.
#
# ── A gate that skips itself is worse than no gate ──────────────────────────
# It reads as a pass. v0.2.16 shipped with the bundle-size gate silently skipped
# because it was hidden behind a `tools/smoke/node_modules` check it never
# needed, and the run still printed ALL GREEN. So: every gate either RUNS or is
# counted a FAILURE, and the closing summary names exactly what was verified and
# what was not. Never add a gate whose "can't run" branch is a bare echo.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

PROFILE_FLAG=""
PROFILE_LABEL="debug"
RUN_SMOKE=0

for arg in "$@"; do
    case "$arg" in
        --release) PROFILE_FLAG="--release"; PROFILE_LABEL="release" ;;
        --smoke) RUN_SMOKE=1 ;;
        -h|--help)
            echo "Usage: $0 [--release] [--smoke]"
            echo "  --release  use --release profile (slower, matches CI)"
            echo "  --smoke    run Playwright smoke (needs website running on :8094)"
            exit 0
            ;;
        *) echo "Unknown arg: $arg" >&2; exit 2 ;;
    esac
done

export PATH="$HOME/.cargo/bin:$PATH"

section() { printf "\n\033[1;36m== %s ==\033[0m\n" "$*"; }
ok()      { printf "\033[1;32mOK\033[0m %s\n" "$*"; }
fail()    { printf "\033[1;31mFAIL\033[0m %s\n" "$*" >&2; }

failures=0
verified=()   # gates that actually ran and passed
notrun=()     # gates deliberately not run this invocation (opt-in only)

section "version parity"
# game/engine/Cargo.toml and tools/packaging/packager.toml MUST agree.
#
# cargo-packager stamps artefact NAMES from packager.toml, and the docs site
# derives the published version from those filenames — so a drift means the
# download page (and /download/latest.json, and therefore the in-game update
# indicator) advertises a version that is not what the binary reports.
#
# This is not hypothetical: v0.2.16 bumped only Cargo.toml, leaving packager.toml
# at 0.2.15. The published AppImage was v0.2.16 code wearing a 0.2.15 label for
# four days, and the in-game indicator would have reported "(dev)" on a
# perfectly current build. Cheap to check, silent and confusing when wrong.
engine_ver="$(grep -m1 '^version = ' game/engine/Cargo.toml | cut -d'"' -f2)"
packager_ver="$(grep -m1 '^version = ' tools/packaging/packager.toml | cut -d'"' -f2)"
if [ -n "$engine_ver" ] && [ "$engine_ver" = "$packager_ver" ]; then
    ok "engine and packager versions agree ($engine_ver)"
    verified+=("version parity")
else
    fail "version drift: game/engine/Cargo.toml=$engine_ver but tools/packaging/packager.toml=$packager_ver"
    failures=$((failures + 1))
fi

section "docs-site unit tests (/download/latest.json contract)"
# tools/sites/docs/versioning.py is deliberately pure (no FastAPI) so this runs
# on a bare python3 — no site venv, so it can never be "skipped in a fresh
# worktree". It is the server half of the contract the native in-game update
# indicator relies on; update_check.rs pins the client half.
if command -v python3 >/dev/null 2>&1; then
    if (cd tools/sites/docs && python3 -m unittest discover -s . -p 'test_*.py' -q); then
        ok "docs-site tests pass"
        verified+=("docs-site unit tests")
    else
        fail "docs-site tests failed"
        failures=$((failures + 1))
    fi
else
    fail "python3 not found — docs-site tests could NOT run"
    failures=$((failures + 1))
fi

section "cargo clippy ($PROFILE_LABEL)"
if (cd game/engine && cargo clippy $PROFILE_FLAG -- -D warnings); then
    ok "clippy clean (-D warnings)"
    verified+=("clippy -D warnings")
else
    fail "clippy errored"
    failures=$((failures + 1))
fi

section "cargo build ($PROFILE_LABEL)"
if (cd game/engine && cargo build $PROFILE_FLAG); then
    ok "native build green"
    verified+=("native build")
else
    fail "native build broken"
    failures=$((failures + 1))
fi

section "licence audit (cargo-deny)"
# Keeps the third-party licence audit current instead of a one-time scan done
# by hand: a future crate bump that pulls in GPL/AGPL/SSPL/unknown now fails
# this gate instead of going unnoticed until someone reads the dependency
# tree by eye. Allow-list lives in tools/packaging/deny.toml (kept in step
# with tools/packaging/about.toml's THIRD-PARTY-NOTICES generator).
# cargo-deny isn't installed on demand (a surprise heavy compile); a missing
# tool fails the gate with the exact pinned install line.
if command -v cargo-deny >/dev/null 2>&1; then
    if (cd game/engine && cargo deny --config "$ROOT/tools/packaging/deny.toml" check licenses); then
        ok "licence audit clean"
        verified+=("cargo deny check licenses")
    else
        fail "disallowed or unrecognised licence in the dependency graph"
        failures=$((failures + 1))
    fi
else
    fail "cargo-deny not installed — run: cargo install cargo-deny --version 0.20.2 --locked"
    failures=$((failures + 1))
fi

section "cargo test ($PROFILE_LABEL)"
# Status comes from the subshell DIRECTLY — no pipeline. The old form was
# `cargo test ... | tail -5`, whose exit status is `tail`'s, and which therefore
# reported success for a failing suite unless `pipefail` happened to be set 40
# lines above. It was correct, but only by action at a distance: deleting one
# word from `set -euo pipefail` would have blinded the entire test gate silently.
# Redirecting to a log and tailing it afterwards removes that coupling — and
# shows MORE output on failure, not less.
test_log="$(mktemp)"
# `--lib`, not `--bin axenstax-engine`: the engine is a lib + thin bin shim so
# Android can build a cdylib (a bin target cannot be dlopen'd by NativeActivity).
# Every #[cfg(test)] module lives in the library, so `--bin` would silently run
# ZERO tests and still pass.
if (cd game/engine && CARGO_INCREMENTAL=0 cargo test --lib $PROFILE_FLAG --quiet) >"$test_log" 2>&1; then
    tail -5 "$test_log"
    # A zero-test "pass" is the failure mode the --bin/--lib switch exists to
    # prevent, so count what actually ran. The suite is well into the hundreds;
    # anything under 100 means the wrong target was tested.
    passed_total="$(awk '/test result: ok\./ { for (i = 1; i <= NF; i++) if ($i == "passed;") s += $(i - 1) } END { print s + 0 }' "$test_log")"
    if [ "${passed_total:-0}" -ge 100 ]; then
        ok "all tests pass ($passed_total)"
        verified+=("cargo test ($passed_total tests)")
    else
        fail "only ${passed_total:-0} tests ran — wrong cargo test target?"
        failures=$((failures + 1))
    fi
else
    tail -40 "$test_log"
    fail "tests failed"
    failures=$((failures + 1))
fi
rm -f "$test_log"

section "trunk build (wasm)"
if (cd game/engine && trunk build); then
    ok "wasm build green"
    verified+=("trunk wasm build")
else
    fail "wasm build broken"
    failures=$((failures + 1))
fi

section "forbidden-symbol gate (web bundle)"
# Proves the web build still carries none of the native-only chat surface (the
# room plug, guardian copy, the policy file, the URL scheme). cfg is what keeps
# them out; this is what proves the cfg still works after a refactor. Node
# built-ins only, so it needs no npm install — and a missing `node` is a
# FAILURE, not a shrug, per this file's own rule at the top.
if command -v node >/dev/null 2>&1; then
    if (cd tools/smoke && node forbidden-symbol.mjs); then
        ok "no native-only chat surface in the web bundle"
        verified+=("forbidden-symbol gate")
    else
        fail "native-only chat surface leaked into the web bundle"
        failures=$((failures + 1))
    fi
else
    fail "node not found — forbidden-symbol gate could NOT run (install Node to gate the bundle)"
    failures=$((failures + 1))
fi

section "bundle size gate (< 5 MB brotli)"
# bundle-size.mjs uses only Node built-ins (node:fs, node:zlib, node:path), so it
# needs NO npm install — it was previously gated behind tools/smoke/node_modules
# and silently skipped in a fresh worktree while the run still printed ALL GREEN.
# Only a missing `node` is a real reason it can't run, and that counts as a
# FAILURE rather than a shrug.
if command -v node >/dev/null 2>&1; then
    if (cd tools/smoke && node bundle-size.mjs); then
        ok "bundle size under gate"
        verified+=("bundle size < 5 MiB brotli")
    else
        fail "bundle size over gate"
        failures=$((failures + 1))
    fi
else
    fail "node not found — bundle size gate could NOT run (install Node to gate the bundle)"
    failures=$((failures + 1))
fi

if [ "$RUN_SMOKE" -eq 1 ]; then
    section "playwright smoke"
    if [ ! -d tools/smoke/node_modules ]; then
        fail "tools/smoke/node_modules missing — run: (cd tools/smoke && npm install && npx playwright install chromium)"
        failures=$((failures + 1))
    elif (cd tools/smoke && node smoke.mjs); then
        ok "smoke passed"
        verified+=("playwright smoke")
    else
        fail "smoke failed"
        failures=$((failures + 1))
    fi
else
    notrun+=("playwright smoke — pass --smoke (needs the game site on :8094)")
fi

# Say what was actually checked. "ALL GREEN" on its own invites being read as
# "everything is verified", which is how a skipped gate slips through unnoticed.
section "what this run verified"
if [ "${#verified[@]}" -gt 0 ]; then
    for g in "${verified[@]}"; do printf "  \033[1;32m✓\033[0m %s\n" "$g"; done
fi
if [ "${#notrun[@]}" -gt 0 ]; then
    for g in "${notrun[@]}"; do printf "  \033[1;33m–\033[0m %s \033[1;33m[NOT RUN]\033[0m\n" "$g"; done
fi

if [ "$failures" -eq 0 ]; then
    if [ "${#notrun[@]}" -gt 0 ]; then
        section "ALL GREEN (${#verified[@]} gates — see NOT RUN above)"
    else
        section "ALL GREEN (${#verified[@]} gates, nothing skipped)"
    fi
    exit 0
else
    section "$failures FAILURE(S)"
    exit 1
fi
