"""Installer-format table and User-Agent OS detection for the /download page.

Deliberately pure (no FastAPI), like `versioning.py`, so `check.sh` can unit-test
it with a bare `python3`. `app.py` imports everything from here.
"""

# extension -> (os key, human format, per-OS expectation, severity). Severity drives
# the warning colour: clean (Linux), warn (Windows SmartScreen / Android Play
# Protect), block (macOS Gatekeeper).
#
# `.apk` is deliberately a sideload, not a Google Play listing: shipping the APK
# ourselves keeps Play review, the Data safety form and Google's crypto policy out
# of the loop. The cost is the two-step install, so the note has to carry it.
INSTALLER_META = {
    ".appimage": ("linux", "AppImage (portable)", "chmod +x and run — no signing gate.", "clean"),
    ".deb": ("linux", ".deb (Debian/Ubuntu)", "Install with: sudo apt install ./<file>.", "clean"),
    ".exe": ("windows", "Installer (.exe)", "Unsigned: SmartScreen → More info → Run anyway.", "warn"),
    ".msi": ("windows", "Installer (.msi)", "Unsigned: SmartScreen → More info → Run anyway.", "warn"),
    ".dmg": ("macos", "Disk image (.dmg)", "Unsigned & un-notarised — macOS blocks it until we notarise (see below).", "block"),
    ".apk": ("android", "APK (arm64 tablet & phone)", "Sideload: allow your browser to install apps, then Play Protect warns — tap More details → Install anyway. Not on Google Play.", "warn"),
}

OS_LABELS = {"linux": "🐧 Linux", "windows": "🪟 Windows", "macos": "🍎 macOS", "android": "🤖 Android"}


def detect_os(user_agent: str) -> str | None:
    """Best-effort OS detection from the User-Agent (iOS → None; there is no iOS build).

    Android must be tested BEFORE Linux: an Android UA reads
    `Mozilla/5.0 (Linux; Android 14; Pixel 8) …`, so a plain "linux" test claims
    it. The Linux branch keeps its own `not android` guard so neither test can
    drift into matching the other.
    """
    ua = (user_agent or "").lower()
    if "android" in ua:
        return "android"
    if "windows" in ua:
        return "windows"
    if ("macintosh" in ua or "mac os x" in ua) and "mobile" not in ua and "iphone" not in ua and "ipad" not in ua:
        return "macos"
    if "linux" in ua and "android" not in ua:
        return "linux"
    return None
