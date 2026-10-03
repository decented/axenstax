"""The published-version contract behind `/download/latest.json`.

Pure — no FastAPI, no filesystem — so it is unit-testable with a bare
`python3` (no site venv) and `check.sh` can gate it on every machine. The
native engine's `game/engine/src/update_check.rs` consumes the document
`latest_manifest` builds; its `the_docs_site_document_shape_parses` test pins
the same shape from the client side. Change the keys here and BOTH tests go red.
"""

from __future__ import annotations

import re

# cargo-packager names artefacts `axenstax-engine_<version>_<arch>.<ext>`, so the
# version is the underscore-delimited field between the name and the arch. This
# is the ONLY place the published version is derived, and it comes from the
# files that actually exist — so /download/latest.json can never advertise a
# version that is not downloadable.
#
# Two shapes must both parse:
#   axenstax-engine_0.2.16_x86_64.AppImage   version followed by an arch field
#   axenstax_0.2.16.apk                      version followed by the extension
# hence the trailing `(?:_|\.[A-Za-z])` alternation rather than a bare `_`.
#
# At least TWO numeric segments are required (`{1,3}`, not `{0,3}`): a bare
# `\d+` would match the "64" in `_x86_64.AppImage` and report it as the version.
INSTALLER_VERSION_RE = re.compile(r"_(\d+(?:\.\d+){1,3})(?=_|\.[A-Za-z])")

# The keys the native client reads (`update_check.rs` reads `version`; the
# rest are for the download page's own consumers and the future updater).
# The last two (added 2026-09-03) are what an in-place AppImage updater
# needs to fetch and verify the new build without re-deriving anything:
# the absolute download URL and the sha256 of that exact file.
MANIFEST_KEYS = (
    "version",
    "linux_appimage",
    "android_apk",
    "download_url",
    "linux_appimage_sha256",
    "linux_appimage_url",
)


def installer_version(filename: str) -> str | None:
    """Extract `0.2.16` from `axenstax-engine_0.2.16_x86_64.AppImage`."""
    m = INSTALLER_VERSION_RE.search(filename)
    return m.group(1) if m else None


def version_sort_key(v: str) -> tuple[int, ...]:
    """Numeric ordering, so 0.2.10 sorts ABOVE 0.2.9 (string ordering does not)."""
    return tuple(int(p) for p in v.split("."))


def filter_newest_per_platform(installers: list[dict]) -> list[dict]:
    """Keep only the newest installer per (os, format), by the version parsed
    from its filename — the SAME `installer_version` parser `latest_manifest`
    uses, so the `/download` page and `/download/latest.json` can never
    disagree about which build is "newest".

    `installers` is a list of dicts each carrying at least "filename", "os"
    and "format" (the shape `_discover_installers()` in app.py builds); this
    only filters, it never mutates an entry or reorders the survivors relative
    to each other (2026-09-27 audit, REVIEW-W6 should-fix #1 — without
    `rsync --delete`, every past release's installer stays on disk and used to
    get listed forever).

    A file whose version can't be parsed (e.g. an unversioned or `-latest-`
    style name) has nothing to compare it against, so it is never deduped
    away — it passes through unchanged."""
    best_version: dict[tuple[str, str], str] = {}
    for inst in installers:
        v = installer_version(inst["filename"])
        if v is None:
            continue
        key = (inst["os"], inst["format"])
        cur = best_version.get(key)
        if cur is None or version_sort_key(v) > version_sort_key(cur):
            best_version[key] = v

    out = []
    for inst in installers:
        v = installer_version(inst["filename"])
        if v is None:
            out.append(inst)
            continue
        key = (inst["os"], inst["format"])
        if v == best_version[key]:
            out.append(inst)
    return out


def latest_manifest(installers: list[dict], download_url: str, installer_url_base: str) -> dict:
    """Build the `/download/latest.json` document from the installers that
    actually exist. `installers` is a list of `{"filename": str, "sha256": str}`
    dicts (the shape `_discover_installers()` in app.py already produces, minus
    the page-rendering fields it doesn't need). `version` is the highest
    version any of them carries, and each per-kind entry names a file OF THAT
    VERSION — a stale older file of one kind never wins, and with no versioned
    installer at all every field but `download_url` is None (never
    `None == None` matching an unversioned file as "the newest").

    `linux_appimage_sha256` / `linux_appimage_url` (added 2026-09-03) are what
    an in-place AppImage updater needs to fetch and verify the newest Linux
    build without re-deriving anything: `linux_appimage_url` is
    `f"{installer_url_base}/{filename}"` — the versioned, user-facing
    `/download/installer/{name}` route, NOT the bare `/download/{name}`
    auto-update-channel route. Both are None whenever `linux_appimage` is."""
    filenames = [i["filename"] for i in installers]
    versions = sorted(
        {v for f in filenames if (v := installer_version(f))},
        key=version_sort_key,
    )
    latest = versions[-1] if versions else None

    def newest_with_suffix(suffix: str) -> str | None:
        if latest is None:
            return None
        matches = [
            f for f in filenames
            if f.lower().endswith(suffix) and installer_version(f) == latest
        ]
        return matches[0] if matches else None

    linux_appimage = newest_with_suffix(".appimage")
    if linux_appimage is None:
        linux_appimage_sha256 = None
        linux_appimage_url = None
    else:
        linux_appimage_sha256 = next(
            i["sha256"] for i in installers if i["filename"] == linux_appimage
        )
        linux_appimage_url = f"{installer_url_base}/{linux_appimage}"

    return {
        "version": latest,
        "linux_appimage": linux_appimage,
        # Stays None until the Android APK ships and `.apk` joins
        # `_INSTALLER_META` in app.py. The client tolerates null.
        "android_apk": newest_with_suffix(".apk"),
        "download_url": download_url,
        "linux_appimage_sha256": linux_appimage_sha256,
        "linux_appimage_url": linux_appimage_url,
    }
