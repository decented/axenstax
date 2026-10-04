#!/usr/bin/env bash
#
# Build the Axe'n'Stax Android APK — WITHOUT Gradle.
#
# The APK contains no Java at all (NativeActivity + hasCode="false"), so the
# whole pipeline is cargo-ndk -> aapt2 link -> zip in the .so -> zipalign ->
# apksigner. Four SDK tools, no Gradle daemon, no build.gradle to drift.
#
# CPU/RAM NOTE: the engine is a large build. The cargo step is `nice`d and
# capped with -j ($JOBS, default 4) so a laptop stays usable. Do NOT run another
# cargo job alongside it.
#
# Usage:
#   ./build-apk.sh                    # debug .so, arm64-v8a
#   PROFILE=release ./build-apk.sh    # release (smaller + much faster on device)
#   ABI=x86_64 ./build-apk.sh         # emulator
#
# Signing a build for DISTRIBUTION (the release key lives outside the repo):
#   ANDROID_KEYSTORE=/path/to/axenstax-release.jks \
#   ANDROID_KEYSTORE_PASS_FILE=/path/to/axenstax-release.password \
#   PROFILE=release ./build-apk.sh
#
# With no ANDROID_KEYSTORE the build self-signs with a throwaway debug key and
# says so. An APK's signing certificate is its PERMANENT identity: Android
# refuses to upgrade an installed app whose signature changed, so a key swap
# forces every player to uninstall — losing their worlds. Publish only ever with
# the one release key, and keep it backed up.
#
# Then:
#   adb install -r out/axenstax.apk
#   adb shell am start -n com.axenstax.game/android.app.NativeActivity
#   adb logcat -s axenstax:V

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
ENGINE="$REPO/game/engine"

# ── Config (all overridable) ────────────────────────────────────────────────
SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
NDK="${ANDROID_NDK_HOME:-$(ls -d "$SDK"/ndk/* 2>/dev/null | sort -V | tail -1)}"
BUILD_TOOLS="${BUILD_TOOLS:-$(ls -d "$SDK"/build-tools/* 2>/dev/null | sort -V | tail -1)}"
PLATFORM="${PLATFORM:-$(ls -d "$SDK"/platforms/* 2>/dev/null | sort -V | tail -1)}"
ABI="${ABI:-arm64-v8a}"
PROFILE="${PROFILE:-debug}"
JOBS="${JOBS:-4}"
NICE="${NICE:-10}"

for tool in aapt2 zipalign apksigner; do
    [ -x "$BUILD_TOOLS/$tool" ] || { echo "ERROR: $tool not in $BUILD_TOOLS" >&2; exit 1; }
done
[ -f "$PLATFORM/android.jar" ] || { echo "ERROR: no android.jar in $PLATFORM" >&2; exit 1; }
[ -d "$NDK" ] || { echo "ERROR: NDK not found (set ANDROID_NDK_HOME)" >&2; exit 1; }

echo "ABI=$ABI PROFILE=$PROFILE JOBS=$JOBS"

OUT="$HERE/out"
STAGE="$OUT/stage"
rm -rf "$OUT"
mkdir -p "$STAGE/lib/$ABI"

# ── 1. Build the cdylib ────────────────────────────────────────────────────
# REQUIRED: -lc++abi. rodio -> cpal -> oboe compiles C++, and cargo-ndk emits
# -lc++_static WITHOUT the matching libc++abi, so the link dies on
# std::terminate / __cxa_guard_acquire / __cxa_guard_release. This mirrors
# game/engine/.cargo/config.toml, which is .gitignore'd and so cannot be relied
# on to exist on a fresh clone.
echo ">>> [1/5] cargo ndk build ($PROFILE)"
export ANDROID_NDK_HOME="$NDK"
export RUSTFLAGS="${RUSTFLAGS:-} -Clink-arg=-lc++abi"

# Keep Android objects OUT of the engine's default target dir, so an Android
# build never invalidates the host/WASM caches (and vice versa) — they use
# different target triples and would otherwise thrash each other on a machine
# where a full rebuild is expensive.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/build-android}"

REL_FLAG=""
[ "$PROFILE" = "release" ] && REL_FLAG="--release"

( cd "$ENGINE" && nice -n "$NICE" cargo ndk -t "$ABI" -o "$STAGE/lib_cargo" \
    build $REL_FLAG -j "$JOBS" )

cp "$STAGE/lib_cargo/$ABI/libaxenstax_engine.so" "$STAGE/lib/$ABI/"
rm -rf "$STAGE/lib_cargo"
echo "    .so size: $(du -h "$STAGE/lib/$ABI/libaxenstax_engine.so" | cut -f1)"

# ── 2. Compile the launcher-icon resources and link the APK ───────────
# versionCode/versionName are INJECTED here rather than hardcoded in the
# manifest, so they can never drift from Cargo.toml.
#
# They are not cosmetic. Android compares versionCode to decide whether an APK
# is an upgrade, and refuses to install an older one over a newer; with none
# declared it reads as empty and every build looks identical, so real updates
# and Charter's guardian grant (which pins version_code) both become
# meaningless. `latest.json` advertises a version the installed app must be able
# to compare itself against.
#
# major*1000000 + minor*1000 + patch keeps the code monotonic across a normal
# semver bump while minor and patch stay under 1000: 0.2.16 -> 2016.
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ENGINE/Cargo.toml" | head -1)"
[ -n "$VERSION" ] || { echo "ERROR: could not read version from $ENGINE/Cargo.toml" >&2; exit 1; }
IFS=. read -r V_MAJ V_MIN V_PAT <<< "$VERSION"
V_PAT="${V_PAT:-0}"
if [ "$V_MIN" -ge 1000 ] || [ "$V_PAT" -ge 1000 ]; then
    echo "ERROR: version $VERSION overflows the versionCode scheme (minor/patch must be < 1000)" >&2
    exit 1
fi
VERSION_CODE=$(( V_MAJ * 1000000 + V_MIN * 1000 + V_PAT ))

echo ">>> [2/5] aapt2 compile res/ + link (versionCode=$VERSION_CODE versionName=$VERSION)"
# res/ holds only the launcher icon (mipmap-*) and its background colour. The APK
# still has no Java/Gradle; aapt2 compiles the resources straight into the package.
"$BUILD_TOOLS/aapt2" compile --dir "$HERE/res" -o "$OUT/res.zip"
"$BUILD_TOOLS/aapt2" link \
    -I "$PLATFORM/android.jar" \
    --manifest "$HERE/AndroidManifest.xml" \
    -R "$OUT/res.zip" \
    --version-code "$VERSION_CODE" \
    --version-name "$VERSION" \
    -o "$OUT/unaligned.apk" \
    --auto-add-overlay

# ── 3. Add the native library ──────────────────────────────────────────────
echo ">>> [3/5] adding lib/$ABI/libaxenstax_engine.so"
( cd "$STAGE" && zip -q -r "$OUT/unaligned.apk" "lib" )

# ── 4. Align ───────────────────────────────────────────────────────────────
echo ">>> [4/5] zipalign"
"$BUILD_TOOLS/zipalign" -f -p 4 "$OUT/unaligned.apk" "$OUT/aligned.apk"

# ── 5. Sign ────────────────────────────────────────────────────────────────
# ANDROID_KEYSTORE selects the real release key (kept OUTSIDE the repo). With it
# unset we self-sign with a throwaway debug key so a dev build still installs.
# The debug keystore is .gitignore'd; the release one must never be in the repo
# at all.
if [ -n "${ANDROID_KEYSTORE:-}" ]; then
    KS="$ANDROID_KEYSTORE"
    [ -f "$KS" ] || { echo "ERROR: ANDROID_KEYSTORE=$KS does not exist" >&2; exit 1; }
    PASS_FILE="${ANDROID_KEYSTORE_PASS_FILE:-}"
    [ -n "$PASS_FILE" ] || { echo "ERROR: ANDROID_KEYSTORE needs ANDROID_KEYSTORE_PASS_FILE" >&2; exit 1; }
    [ -f "$PASS_FILE" ] || { echo "ERROR: pass file $PASS_FILE does not exist" >&2; exit 1; }
    # env: (not pass:) so the password never lands in the process list where any
    # other user could read it off `ps`.
    #
    # NOT file: — apksigner's PasswordRetriever reads each `file:` spec
    # SEQUENTIALLY from the same open handle, so passing one file for both
    # --ks-pass and --key-pass consumes the only line on the first read and the
    # second dies with "end of file reached". env: is read fresh each time.
    # (The keystore is PKCS12, where the key password always equals the store
    # password, so one secret legitimately serves both.)
    export AXENSTAX_KS_PASS="$(cat "$PASS_FILE")"
    [ -n "$AXENSTAX_KS_PASS" ] || { echo "ERROR: $PASS_FILE is empty" >&2; exit 1; }
    KS_PASS="env:AXENSTAX_KS_PASS"
    KEY_ALIAS="${ANDROID_KEY_ALIAS:-axenstax}"
    SIGNED_WITH="release key ($(basename "$KS"))"
else
    KS="$HERE/debug.keystore"
    if [ ! -f "$KS" ]; then
        echo ">>> generating debug keystore"
        keytool -genkeypair -v -keystore "$KS" -storepass android -keypass android \
            -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 \
            -dname "CN=Android Debug,O=Android,C=US" >/dev/null
    fi
    KS_PASS="pass:android"
    KEY_ALIAS="androiddebugkey"
    SIGNED_WITH="THROWAWAY DEBUG KEY — not distributable"
fi

# Versioned, arch-tagged name matching the AppImage convention
# (axenstax-engine_0.2.16_x86_64.AppImage). The docs site parses the version out
# of the filename to build /download/latest.json, so a bare `axenstax.apk` would
# publish with a null version — see _INSTALLER_VERSION_RE in tools/sites/docs/app.py.
# ($VERSION was read from Cargo.toml in step 2, where it also sets versionCode.)
APK="$OUT/axenstax-engine_${VERSION}_${ABI}.apk"

echo ">>> [5/5] apksigner ($SIGNED_WITH)"
"$BUILD_TOOLS/apksigner" sign \
    --ks "$KS" --ks-pass "$KS_PASS" --key-pass "$KS_PASS" \
    --ks-key-alias "$KEY_ALIAS" \
    --out "$APK" "$OUT/aligned.apk"

rm -f "$OUT/unaligned.apk" "$OUT/aligned.apk" "$OUT/res.zip" "$APK.idsig"
rm -rf "$STAGE"

# Stable name for the documented adb workflow, so the recipe keeps working while
# the published file stays versioned.
ln -sf "$(basename "$APK")" "$OUT/axenstax.apk"

echo
echo "BUILT: $APK ($(du -h "$APK" | cut -f1))"
echo "SIGNED WITH: $SIGNED_WITH"
"$BUILD_TOOLS/apksigner" verify --print-certs "$APK" | grep -i 'SHA-256 digest\|DN:' || true
echo
echo "  adb install -r $OUT/axenstax.apk"
echo "  adb shell am start -n com.axenstax.game/android.app.NativeActivity"
echo "  adb logcat -s axenstax:V"
