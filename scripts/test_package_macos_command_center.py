#!/usr/bin/env python3
"""Packaging fixtures prove byte preservation, not Apple signature validity."""

from pathlib import Path
import tempfile
import unittest

from package_macos_command_center import BUNDLE_NAME, stage

FIXTURE_PLIST = """<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDisplayName</key>
	<string>AOS Command Center</string>
	<key>CFBundleExecutable</key>
	<string>aos-tray</string>
	<key>CFBundleIdentifier</key>
	<string>ai.unicity.aos.tray</string>
	<key>CFBundleName</key>
	<string>AOS Command Center</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>2026.9.2</string>
</dict>
</plist>
"""


def fixture(app: Path) -> None:
    info = app / "Contents" / "Info.plist"
    binary = app / "Contents" / "MacOS" / "aos-tray"
    signature = app / "Contents" / "_CodeSignature" / "CodeResources"
    info.parent.mkdir(parents=True, exist_ok=True)
    binary.parent.mkdir(parents=True, exist_ok=True)
    signature.parent.mkdir(parents=True, exist_ok=True)
    info.write_text(FIXTURE_PLIST)
    info.chmod(0o644)
    binary.write_bytes(b"fixture-only:aos-tray")
    binary.chmod(0o755)
    signature.write_bytes(b"fixture-only:CodeResources")
    signature.chmod(0o644)


class PackagingTests(unittest.TestCase):
    def test_preserves_every_member_and_mode(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "supplied.app"
            output = root / "share"
            fixture(source)
            stage(source, output)
            copied = output / BUNDLE_NAME
            self.assertTrue(copied.is_dir())
            for path in source.rglob("*"):
                if path.is_file():
                    staged = copied / path.relative_to(source)
                    self.assertEqual(path.read_bytes(), staged.read_bytes())
                    self.assertEqual(path.stat().st_mode, staged.stat().st_mode)

    def test_missing_required_app(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(ValueError):
                stage(root / "missing.app", root / "share")

    def test_rejects_missing_signature_and_links(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "supplied.app"
            fixture(source)
            signature = source / "Contents/_CodeSignature/CodeResources"
            signature.unlink()
            with self.assertRaises(ValueError):
                stage(source, root / "share")
            signature.symlink_to(root / "outside")
            with self.assertRaises(ValueError):
                stage(source, root / "share")

    def test_rejects_wrong_bundle_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "supplied.app"
            fixture(source)
            (source / "Contents/Info.plist").write_text(
                FIXTURE_PLIST.replace("ai.unicity.aos.tray", "ai.example.other")
            )
            with self.assertRaises(ValueError):
                stage(source, root / "share")


if __name__ == "__main__":
    unittest.main()
