#!/usr/bin/env python3
"""Fast regression tests for the icon checks, independent of the Android SDK."""
import tempfile
import unittest
from pathlib import Path

import generate_launcher_icons as generator
from verify_launcher_icons import check_manifest


class LauncherChecks(unittest.TestCase):
    def setUp(self):
        self.manifest = (generator.ROOT / "app/src/main/AndroidManifest.xml").read_text()

    def verify_text(self, text):
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "AndroidManifest.xml"
            manifest.write_text(text)
            return check_manifest(manifest)

    def test_launcher_inherits_application_icon(self):
        self.assertEqual(self.verify_text(self.manifest), [".MainActivity"])

    def test_missing_icon_is_rejected(self):
        with self.assertRaises(AssertionError):
            self.verify_text(self.manifest.replace('android:icon="@mipmap/ic_launcher"', ""))

    def test_missing_round_icon_is_rejected(self):
        with self.assertRaises(AssertionError):
            self.verify_text(self.manifest.replace('android:roundIcon="@mipmap/ic_launcher_round"', ""))

    def test_unexpected_launcher_override_is_rejected(self):
        with self.assertRaises(AssertionError):
            self.verify_text(self.manifest.replace('<activity android:name=', '<activity android:icon="@mipmap/wrong" android:name='))

    def test_missing_launcher_is_rejected(self):
        with self.assertRaises(AssertionError):
            self.verify_text(self.manifest.replace("android.intent.category.LAUNCHER", "android.intent.category.DEFAULT"))

    def test_pinned_exports_match_checked_in_bytes(self):
        outputs, provenance = generator.build()
        for path, content in outputs.items():
            self.assertEqual((generator.RES / path).read_bytes(), content, path)
        self.assertEqual((generator.ROOT / "branding/launcher-provenance.json").read_bytes(), provenance)


if __name__ == "__main__":
    unittest.main()
