"""Unit tests for the `/download/latest.json` contract. Run by `check.sh`
with a bare `python3` — no venv, no FastAPI — so the gate can never skip."""

import unittest

from versioning import (
    MANIFEST_KEYS,
    filter_newest_per_platform,
    installer_version,
    latest_manifest,
    version_sort_key,
)

DL = "https://docs.axenstax.org/download"
INSTALLER_BASE = "https://docs.axenstax.org/download/installer"


def _installers(*filenames: str) -> list[dict]:
    """Build the `{"filename", "sha256"}` dicts `latest_manifest` now expects,
    with a distinct fake sha256 per filename so tests can tell them apart."""
    return [{"filename": f, "sha256": f"sha-of-{f}"} for f in filenames]


class InstallerVersion(unittest.TestCase):
    def test_appimage_name_with_arch_field(self):
        self.assertEqual(installer_version("axenstax-engine_0.2.16_x86_64.AppImage"), "0.2.16")

    def test_apk_name_with_no_arch_field(self):
        self.assertEqual(installer_version("axenstax_0.2.16.apk"), "0.2.16")

    def test_the_64_in_x86_64_is_not_a_version(self):
        # `_64.AppImage` must not be read as version "64" when the real version
        # is absent — that would advertise a build that does not exist.
        self.assertIsNone(installer_version("axenstax-engine_x86_64.AppImage"))

    def test_unversioned_names_yield_none(self):
        for name in ("AxeNStax.AppImage", "axenstax-engine-latest-x86_64.AppImage", "", "readme.txt"):
            self.assertIsNone(installer_version(name), name)


class FilterNewestPerPlatform(unittest.TestCase):
    """/download must list only the newest version per (os, format) — without
    `rsync --delete`, every past release's installer stays on disk
    (2026-09-27 audit, REVIEW-W6 should-fix #1)."""

    @staticmethod
    def _inst(filename: str, os_key: str, fmt: str) -> dict:
        return {"filename": filename, "os": os_key, "format": fmt}

    def test_older_version_of_same_platform_format_is_dropped(self):
        installers = [
            self._inst("axenstax-engine_0.2.26_x86_64.AppImage", "linux", "AppImage (portable)"),
            self._inst("axenstax-engine_0.2.27_x86_64.AppImage", "linux", "AppImage (portable)"),
        ]
        out = filter_newest_per_platform(installers)
        self.assertEqual([i["filename"] for i in out], ["axenstax-engine_0.2.27_x86_64.AppImage"])

    def test_different_formats_of_the_same_os_are_independent(self):
        # .AppImage and .deb are both "linux" but distinct formats — each keeps
        # its own newest, one dropping older files never touches the other.
        installers = [
            self._inst("axenstax-engine_0.2.26_x86_64.AppImage", "linux", "AppImage (portable)"),
            self._inst("axenstax-engine_0.2.27_x86_64.AppImage", "linux", "AppImage (portable)"),
            self._inst("axenstax-engine_0.2.25_amd64.deb", "linux", ".deb (Debian/Ubuntu)"),
        ]
        out = filter_newest_per_platform(installers)
        self.assertEqual(
            sorted(i["filename"] for i in out),
            sorted(["axenstax-engine_0.2.27_x86_64.AppImage", "axenstax-engine_0.2.25_amd64.deb"]),
        )

    def test_partial_release_keeps_each_platforms_own_newest(self):
        # A linux-only publish must not make Windows/macOS vanish — each OS's
        # own newest survives independently even when only linux just shipped.
        installers = [
            self._inst("axenstax-engine_0.2.27_x86_64.AppImage", "linux", "AppImage (portable)"),
            self._inst("AxeNStax-0.2.26-win.exe", "windows", "Installer (.exe)"),
            self._inst("AxeNStax-0.2.26.dmg", "macos", "Disk image (.dmg)"),
        ]
        out = filter_newest_per_platform(installers)
        self.assertEqual(
            sorted(i["filename"] for i in out),
            sorted([
                "axenstax-engine_0.2.27_x86_64.AppImage",
                "AxeNStax-0.2.26-win.exe",
                "AxeNStax-0.2.26.dmg",
            ]),
        )

    def test_unversioned_filenames_pass_through_unfiltered(self):
        installers = [
            self._inst("AxeNStax.AppImage", "linux", "AppImage (portable)"),
            self._inst("axenstax-engine_0.2.27_x86_64.AppImage", "linux", "AppImage (portable)"),
        ]
        out = filter_newest_per_platform(installers)
        self.assertEqual(
            sorted(i["filename"] for i in out),
            sorted(["AxeNStax.AppImage", "axenstax-engine_0.2.27_x86_64.AppImage"]),
        )

    def test_empty_input(self):
        self.assertEqual(filter_newest_per_platform([]), [])


class VersionOrdering(unittest.TestCase):
    def test_numeric_not_lexical(self):
        self.assertGreater(version_sort_key("0.2.10"), version_sort_key("0.2.9"))
        self.assertGreater(version_sort_key("1.0.0"), version_sort_key("0.99.99"))


class LatestManifest(unittest.TestCase):
    def test_document_shape_matches_what_the_native_client_parses(self):
        # Mirror of update_check.rs::the_docs_site_document_shape_parses.
        doc = latest_manifest(
            _installers("axenstax-engine_0.2.18_x86_64.AppImage"), DL, INSTALLER_BASE
        )
        self.assertEqual(tuple(doc.keys()), MANIFEST_KEYS)
        self.assertEqual(doc["version"], "0.2.18")
        self.assertEqual(doc["linux_appimage"], "axenstax-engine_0.2.18_x86_64.AppImage")
        self.assertIsNone(doc["android_apk"])
        self.assertEqual(doc["download_url"], DL)

    def test_highest_version_wins_numerically(self):
        doc = latest_manifest(
            _installers(
                "axenstax-engine_0.2.9_x86_64.AppImage",
                "axenstax-engine_0.2.10_x86_64.AppImage",
            ),
            DL, INSTALLER_BASE,
        )
        self.assertEqual(doc["version"], "0.2.10")
        self.assertEqual(doc["linux_appimage"], "axenstax-engine_0.2.10_x86_64.AppImage")

    def test_a_stale_older_file_of_one_kind_never_wins(self):
        # Only an APK at 0.2.17 is published alongside a 0.2.18 AppImage: the
        # manifest must not name the older APK as if it were 0.2.18.
        doc = latest_manifest(
            _installers(
                "axenstax-engine_0.2.18_x86_64.AppImage",
                "axenstax_0.2.17.apk",
            ),
            DL, INSTALLER_BASE,
        )
        self.assertEqual(doc["version"], "0.2.18")
        self.assertIsNone(doc["android_apk"])

    def test_partial_platform_set_still_reports_the_newest_linux_build(self):
        # Regression (2026-09-27 audit): a routine linux_only installer publish
        # must never require a matching Windows/macOS installer to be present —
        # publish-installers.yml's rsync used to `--delete` the box's installers
        # dir, wiping the *other* OSes' files on every ordinary linux-only run.
        # The fix (dropping --delete) means the on-disk set is a MERGE of
        # whatever ran before plus this run's files — so the manifest must read
        # correctly from a mixed/partial set, exactly like this.
        doc = latest_manifest(
            _installers(
                "axenstax-engine_0.2.18_x86_64.AppImage",
                "AxeNStax-0.2.17-win.exe",
                "AxeNStax-0.2.17.dmg",
            ),
            DL, INSTALLER_BASE,
        )
        self.assertEqual(doc["version"], "0.2.18")
        self.assertEqual(doc["linux_appimage"], "axenstax-engine_0.2.18_x86_64.AppImage")

    def test_no_installers_at_all(self):
        doc = latest_manifest([], DL, INSTALLER_BASE)
        self.assertEqual(doc, {
            "version": None,
            "linux_appimage": None,
            "android_apk": None,
            "download_url": DL,
            "linux_appimage_sha256": None,
            "linux_appimage_url": None,
        })

    def test_unversioned_files_never_match_a_null_version(self):
        # Regression: `installer_version(f) == latest` was `None == None` → True,
        # so an unversioned AppImage was named "newest" beside `"version": null`.
        doc = latest_manifest(_installers("AxeNStax.AppImage"), DL, INSTALLER_BASE)
        self.assertIsNone(doc["version"])
        self.assertIsNone(doc["linux_appimage"])
        self.assertIsNone(doc["linux_appimage_sha256"])
        self.assertIsNone(doc["linux_appimage_url"])


class LatestManifestAppimageChecksum(unittest.TestCase):
    """The two fields an in-place AppImage updater needs (added 2026-09-03)."""

    def test_sha256_and_url_present_and_matching_the_named_appimage(self):
        name = "axenstax-engine_0.2.18_x86_64.AppImage"
        doc = latest_manifest(_installers(name, "axenstax_0.2.18.apk"), DL, INSTALLER_BASE)
        self.assertEqual(doc["linux_appimage"], name)
        self.assertEqual(doc["linux_appimage_sha256"], f"sha-of-{name}")
        self.assertEqual(doc["linux_appimage_url"], f"{INSTALLER_BASE}/{name}")

    def test_both_none_when_no_appimage(self):
        doc = latest_manifest(_installers("axenstax_0.2.18.apk"), DL, INSTALLER_BASE)
        self.assertIsNone(doc["linux_appimage"])
        self.assertIsNone(doc["linux_appimage_sha256"])
        self.assertIsNone(doc["linux_appimage_url"])

    def test_url_uses_the_installer_download_route(self):
        # NOT the bare /download/<file> route — that one is scoped to the
        # -latest- auto-update channel only, and 404s on a versioned filename.
        name = "axenstax-engine_0.2.18_x86_64.AppImage"
        doc = latest_manifest(_installers(name), DL, INSTALLER_BASE)
        self.assertEqual(doc["linux_appimage_url"], f"{INSTALLER_BASE}/{name}")
        self.assertTrue(doc["linux_appimage_url"].startswith(f"{INSTALLER_BASE}/"))

    def test_key_order_unchanged_for_the_first_four_keys(self):
        doc = latest_manifest([], DL, INSTALLER_BASE)
        self.assertEqual(
            tuple(doc.keys())[:4],
            ("version", "linux_appimage", "android_apk", "download_url"),
        )
        self.assertEqual(
            tuple(doc.keys())[4:],
            ("linux_appimage_sha256", "linux_appimage_url"),
        )


if __name__ == "__main__":
    unittest.main()
