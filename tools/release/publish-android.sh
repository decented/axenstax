#!/usr/bin/env bash
# Publish an Axe'n'Stax ANDROID release (kind-30063, d = axenstax-android-apk) with
# nostr-release-kit's publisher and tools/release/release-android.json (channel, package,
# certificate pin, release-key location, relays, Blossom mirror). Unlike the AppImage
# channel (publish-release.mjs), this event carries version_code and cert, which the
# other games' "More games" catalogue requires. Usage:
#   tools/release/publish-android.sh <apk> --version X.Y.Z [--notes "..."] [--dry-run] [--allow-cert-change] [--allow-older]
#   tools/release/publish-android.sh --print-pubkey
# --dry-run signs with a throwaway key, uploads nothing and publishes nothing (it only READS
# the relays to see what the channel carries). Without it: uploads the APK to the mirror
# in release-android.json, then publishes the event to its relays, signed by the release key
# in $AXENSTAX_RELEASE_KEY_FILE (default ~/.axenstax-release/release-key.hex).
# The versionCode is derived from --version (major * 1e6 + minor * 1e3 + patch), exactly as
# tools/packaging/android/build-apk.sh stamps it into the APK.
# The kit's checkout: $KIT, default ../forgesworn/nostr-release-kit beside this repo
# (install its publisher's dependencies once: `npm ci` in its tools/), at the pinned revision
# below. Env names are the kit's: RELEASE_KIT_RELAYS, RELEASE_KIT_APKSIGNER, RELEASE_KIT_AAPT2.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KIT="${KIT:-$HERE/../../../forgesworn/nostr-release-kit}"
KIT_REV="1c16b3e49433e635263b73e0efd600e8fbe647b3"
[ -f "$KIT/tools/publish-release.mjs" ] || { echo "ERROR: nostr-release-kit not found at $KIT (set KIT)" >&2; exit 1; }
KIT_HEAD="$(git -C "$KIT" rev-parse HEAD 2>/dev/null || true)"
[ "$KIT_HEAD" = "$KIT_REV" ] || { echo "ERROR: $KIT is at ${KIT_HEAD:-?}, this script pins nostr-release-kit $KIT_REV (git -C \"$KIT\" checkout $KIT_REV)" >&2; exit 1; }
# ...and match it exactly: no uncommitted or untracked files in the parts of the kit this uses.
KIT_DIRTY="$(git -C "$KIT" status --porcelain -- android vectors tools 2>&1 || echo "git status failed")"
[ -z "$KIT_DIRTY" ] || { printf 'ERROR: %s has local changes; publish only from a clean checkout of %s:\n%s\n' "$KIT" "$KIT_REV" "$KIT_DIRTY" >&2; exit 1; }
exec node "$KIT/tools/publish-release.mjs" --config "$HERE/release-android.json" "$@"
