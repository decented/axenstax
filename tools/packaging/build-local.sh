#!/usr/bin/env bash
# Build the Linux installers (AppImage + .deb) locally from the engine release binary.
# This is the solo-verifiable half of Track A: Linux signs/packages cleanly with no
# accounts. Windows/macOS go through the CI workflow (native-packages.yml).
#
#   tools/packaging/build-local.sh [formats...]   # default: appimage
#
# Output lands in tools/packaging/staging/ (gitignored).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PKG_DIR="$ROOT/tools/packaging"
STAGING="$PKG_DIR/staging"
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/build}"
# linuxdeploy/appimagetool run with --appimage-extract-and-run, which unpacks
# into $TMPDIR and executes from there. On hosts where /tmp is mounted noexec
# (this laptop: tmpfs nosuid,nodev,noexec) that dies with "Permission denied",
# so point TMPDIR at the build tree, which is executable.
export TMPDIR="${TMPDIR:-$CARGO_TARGET_DIR/tmp}"; mkdir -p "$TMPDIR"

FORMATS=("${@:-appimage}")

echo "== building engine release binary =="
(cd "$ROOT/game/engine" && cargo build --release)
BIN="$CARGO_TARGET_DIR/release/axenstax-engine"
[ -x "$BIN" ] || { echo "engine binary not found at $BIN" >&2; exit 1; }

echo "== staging binary into $STAGING =="
rm -rf "$STAGING"
mkdir -p "$STAGING"
cp "$BIN" "$STAGING/axenstax-engine"

echo "== generating THIRD-PARTY-NOTICES.txt =="
# cargo-about isn't assumed to be installed on every dev machine — install it
# on demand, same as native-packages.yml does in CI.
# MUST land in $PKG_DIR itself, not $STAGING: cargo-packager chdirs into the
# config file's directory before resolving relative `resources` globs (same
# reason `icons` in packager.toml is relative to that directory, not --out-dir).
command -v cargo-about >/dev/null 2>&1 || cargo install cargo-about --locked
(cd "$ROOT/game/engine" && cargo about generate \
  "$PKG_DIR/about.hbs" -c "$PKG_DIR/about.toml" \
  -o "$PKG_DIR/THIRD-PARTY-NOTICES.txt")

echo "== packaging: ${FORMATS[*]} =="
# cargo-packager's -f takes one value per flag (Vec, no delimiter) — give each format its
# own -f, or it reads the 2nd as a subcommand.
fmt_args=()
for f in "${FORMATS[@]}"; do fmt_args+=(-f "$f"); done
cargo packager -c "$PKG_DIR/packager.toml" -o "$STAGING" "${fmt_args[@]}" --verbose

# --- AppImage auto-update: embed zsync update-info + emit the .zsync companion ---
# cargo-packager (0.11.x) does NOT embed AppImage update information, so re-stamp the
# built AppImage with appimagetool — it writes the .upd_info ELF section AND emits the
# .zsync. The embedded URL is frozen, so it points at a STABLE `-latest-` name; publish
# BOTH the versioned AppImage and the `-latest-` AppImage + .zsync to /download.
# Recipe verified on a real AppImage 2026-06-18. Spec:
# docs/superpowers/specs/2026-06-18-appimage-auto-update-design.md
SRC_APPIMG=$(find "$STAGING" -maxdepth 2 -name '*_x86_64.AppImage' | head -1)
if [ -n "$SRC_APPIMG" ]; then
  echo "== AppImage auto-update: embedding zsync update-info =="
  UPD='zsync|https://docs.axenstax.org/download/axenstax-engine-latest-x86_64.AppImage.zsync'
  TOOLS="$CARGO_TARGET_DIR/appimage-tools"; mkdir -p "$TOOLS"
  AT="$TOOLS/appimagetool"
  # Pinned + sha256-verified (2026-09-27 audit fix, matches native-packages.yml):
  # AppImageKit only publishes appimagetool under the "continuous" tag, which
  # upstream moves on every build. Pin this asset's sha256 (last verified
  # 2026-09-27) so a moved or compromised tag is rejected instead of silently
  # run against a locally-cached copy.
  APPIMAGETOOL_SHA256=b90f4a8b18967545fda78a445b27680a1642f1ef9488ced28b65398f2be7add2
  if [ ! -x "$AT" ] || ! echo "${APPIMAGETOOL_SHA256}  $AT" | sha256sum -c - >/dev/null 2>&1; then
    echo "  fetching appimagetool…"
    curl -fsSL https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage -o "$AT"
    echo "${APPIMAGETOOL_SHA256}  $AT" | sha256sum -c -
    chmod +x "$AT"
  fi
  ( cd "$STAGING"
    rm -rf squashfs-root
    "$SRC_APPIMG" --appimage-extract >/dev/null
    ARCH=x86_64 "$AT" --appimage-extract-and-run -u "$UPD" squashfs-root axenstax-engine-latest-x86_64.AppImage >/dev/null 2>&1
    rm -rf squashfs-root )
  GOT=$(readelf -p .upd_info "$STAGING/axenstax-engine-latest-x86_64.AppImage" 2>/dev/null | grep -o 'zsync|.*' | head -1 || true)
  if [ "$GOT" = "$UPD" ] && [ -f "$STAGING/axenstax-engine-latest-x86_64.AppImage.zsync" ]; then
    echo "  OK — update-info embedded + .zsync emitted (publish BOTH the versioned and -latest- files)"
  else
    echo "  WARN — update-info embed failed (embedded='$GOT')" >&2
  fi
fi

echo "== artifacts =="
find "$STAGING" -maxdepth 2 \( -name "*.AppImage" -o -name "*.zsync" -o -name "*.deb" \) -print
