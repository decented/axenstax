"""Unit tests for the /download page's OS detection and installer table.
Bare `python3`, no FastAPI — run by `check.sh`."""

import unittest

from platforms import INSTALLER_META, OS_LABELS, detect_os
from versioning import installer_version

PIXEL = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0 Mobile Safari/537.36"
TABLET = "Mozilla/5.0 (Linux; Android 13; SM-X200) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0 Safari/537.36"
LINUX = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0 Safari/537.36"
WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0 Safari/537.36"
MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"
IPHONE = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1"
IPAD = "Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1"


class DetectOs(unittest.TestCase):
    def test_android_phone_and_tablet_are_android_not_linux(self):
        # Android UAs contain "Linux" — the Android test must win.
        self.assertEqual(detect_os(PIXEL), "android")
        self.assertEqual(detect_os(TABLET), "android")

    def test_desktop_platforms_unchanged(self):
        self.assertEqual(detect_os(LINUX), "linux")
        self.assertEqual(detect_os(WINDOWS), "windows")
        self.assertEqual(detect_os(MAC), "macos")

    def test_ios_and_empty_have_no_build(self):
        for ua in (IPHONE, IPAD, "", None):
            self.assertIsNone(detect_os(ua), ua)


class InstallerTable(unittest.TestCase):
    def test_apk_row_is_android_and_labelled(self):
        os_key, _fmt, note, severity = INSTALLER_META[".apk"]
        self.assertEqual(os_key, "android")
        self.assertEqual(severity, "warn")
        self.assertIn("Install anyway", note)
        self.assertIn("android", OS_LABELS)

    def test_every_row_has_an_os_label(self):
        for ext, (os_key, *_rest) in INSTALLER_META.items():
            self.assertIn(os_key, OS_LABELS, ext)

    def test_build_apk_output_name_carries_a_version(self):
        # build-apk.sh writes axenstax-engine_<version>_<abi>.apk; the
        # latest.json contract reads the version off that name.
        self.assertEqual(installer_version("axenstax-engine_0.2.27_arm64-v8a.apk"), "0.2.27")


if __name__ == "__main__":
    unittest.main()
