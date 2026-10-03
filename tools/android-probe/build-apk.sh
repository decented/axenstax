#!/usr/bin/env bash
#
# Build the Axe'n'Stax GPU probe APK — WITHOUT Gradle.
#
# The APK contains no Java at all (NativeActivity + hasCode="false"), so the
# whole pipeline is: cargo-ndk -> aapt2 link -> zip in the .so -> zipalign ->
# apksigner. That is four SDK tools and no 400 MB Gradle daemon.
#
# CPU NOTE: this host can crash under a fully-maxed CPU, so the cargo step is
# `nice`d and capped with -j. Do NOT run another cargo job alongside it.
#
# Usage:
#   ./build-apk.sh                  # build for arm64-v8a (real phones)
#   ABI=x86_64 ./build-apk.sh       # build for the emulator
#
# Then:
#   adb install -r out/axeprobe.apk
#   adb shell am start -n com.axenstax.probe/android.app.NativeActivity
#   adb logcat -s axeprobe:V

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$HERE"

# ── Config (all overridable) ────────────────────────────────────────────────
SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
NDK="${ANDROID_NDK_HOME:-$(ls -d "$SDK"/ndk/* 2>/dev/null | sort -V | tail -1)}"
BUILD_TOOLS="${BUILD_TOOLS:-$(ls -d "$SDK"/build-tools/* 2>/dev/null | sort -V | tail -1)}"
PLATFORM="${PLATFORM:-$(ls -d "$SDK"/platforms/* 2>/dev/null | sort -V | tail -1)}"
ABI="${ABI:-arm64-v8a}"
JOBS="${JOBS:-4}"          # half this laptop's 12 cores — leaves headroom
NICE="${NICE:-10}"

for tool in aapt2 zipalign apksigner; do
    [ -x "$BUILD_TOOLS/$tool" ] || { echo "ERROR: $tool not found in $BUILD_TOOLS" >&2; exit 1; }
done
[ -f "$PLATFORM/android.jar" ] || { echo "ERROR: android.jar not found in $PLATFORM" >&2; exit 1; }
[ -d "$NDK" ] || { echo "ERROR: NDK not found (set ANDROID_NDK_HOME)" >&2; exit 1; }

echo "SDK=$SDK"
echo "NDK=$NDK"
echo "BUILD_TOOLS=$BUILD_TOOLS"
echo "PLATFORM=$PLATFORM"
echo "ABI=$ABI  JOBS=$JOBS (nice $NICE)"

OUT="$HERE/out"
STAGE="$OUT/stage"
rm -rf "$OUT"
mkdir -p "$STAGE/lib/$ABI"

# ── 1. Compile the Rust cdylib ─────────────────────────────────────────────
echo ">>> [1/5] cargo ndk build (single job, throttled)"
export ANDROID_NDK_HOME="$NDK"
nice -n "$NICE" cargo ndk -t "$ABI" -o "$STAGE/lib_cargo" build --release -j "$JOBS"

# cargo-ndk writes <out>/<abi>/lib<name>.so — move it to the APK's lib/<abi>/.
cp "$STAGE/lib_cargo/$ABI/libaxeprobe.so" "$STAGE/lib/$ABI/libaxeprobe.so"
rm -rf "$STAGE/lib_cargo"
echo "    .so size: $(du -h "$STAGE/lib/$ABI/libaxeprobe.so" | cut -f1)"

# ── 2. Link the (resource-less) APK from the manifest ──────────────────────
echo ">>> [2/5] aapt2 link"
"$BUILD_TOOLS/aapt2" link \
    -I "$PLATFORM/android.jar" \
    --manifest "$HERE/AndroidManifest.xml" \
    -o "$OUT/unaligned.apk" \
    --auto-add-overlay

# ── 3. Add the native library ──────────────────────────────────────────────
echo ">>> [3/5] adding lib/$ABI/libaxeprobe.so"
( cd "$STAGE" && zip -q -r "$OUT/unaligned.apk" "lib" )

# ── 4. Align ───────────────────────────────────────────────────────────────
echo ">>> [4/5] zipalign"
"$BUILD_TOOLS/zipalign" -f -p 4 "$OUT/unaligned.apk" "$OUT/aligned.apk"

# ── 5. Sign (debug key, generated on first run) ────────────────────────────
KS="$HERE/debug.keystore"
if [ ! -f "$KS" ]; then
    echo ">>> generating debug keystore"
    keytool -genkeypair -v -keystore "$KS" -storepass android -keypass android \
        -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 \
        -dname "CN=Android Debug,O=Android,C=US" >/dev/null
fi

echo ">>> [5/5] apksigner"
"$BUILD_TOOLS/apksigner" sign \
    --ks "$KS" --ks-pass pass:android --key-pass pass:android \
    --out "$OUT/axeprobe.apk" "$OUT/aligned.apk"

rm -f "$OUT/unaligned.apk" "$OUT/aligned.apk" "$OUT/axeprobe.apk.idsig"
rm -rf "$STAGE"

echo
echo "BUILT: $OUT/axeprobe.apk ($(du -h "$OUT/axeprobe.apk" | cut -f1))"
echo
echo "Next:"
echo "  adb install -r $OUT/axeprobe.apk"
echo "  adb shell am start -n com.axenstax.probe/android.app.NativeActivity"
echo "  adb logcat -s axeprobe:V"
