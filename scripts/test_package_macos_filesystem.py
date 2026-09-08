#!/usr/bin/env python3
"""Packaging fixtures prove byte preservation, not Apple signature validity."""

from pathlib import Path
import os
import subprocess
import tempfile
import unittest

from package_macos_filesystem import stage


def fixture(root: Path) -> None:
    for prefix, binary in (
        ("AstridFS.app", "AstridFS"),
        ("AstridFS.app/Contents/Extensions/AstridFSAppEx.appex", "AstridFSAppEx"),
    ):
        for name in ("Info.plist", f"MacOS/{binary}", "_CodeSignature/CodeResources"):
            path = root / prefix / "Contents" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(f"fixture-only:{prefix}:{name}".encode())
            path.chmod(0o755 if name.startswith("MacOS/") else 0o644)
    for name in ("manage-macos-fskit.sh", "validate-macos-fskit.sh"):
        path = root / "macos" / name
        path.parent.mkdir(exist_ok=True)
        path.write_text('#!/bin/sh\n[ -z "${AOS_TEST_FSKIT_LOG:-}" ] || printf "%s|%s\\n" "$ASTRID_FSKIT_APP_DEST" "$1" >> "$AOS_TEST_FSKIT_LOG"\n')
        path.chmod(0o755)


class PackagingTests(unittest.TestCase):
    def test_preserves_every_member_and_mode(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, output = root / "source", root / "output"
            fixture(source)
            stage(source, output, True)
            for path in source.rglob("*"):
                if path.is_file():
                    copied = output / path.relative_to(source)
                    self.assertEqual(path.read_bytes(), copied.read_bytes())
                    self.assertEqual(path.stat().st_mode, copied.stat().st_mode)
            log = root / "calls"
            subprocess.run(["/bin/sh", str(output / "macos/aos-filesystem.sh"), "install"],
                           check=True, env={**os.environ, "AOS_TEST_FSKIT_LOG": str(log),
                                            "ASTRID_FSKIT_APP_DEST": str(root / "AOS.app")})
            self.assertEqual(log.read_text(), f"{root}/AOS.app|install\n")

    def test_missing_required_app(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(ValueError):
                stage(root / "source", root / "output", True)
            stage(root / "source", root / "output", False)
            self.assertFalse((root / "output").exists())

    def test_existing_destination_validation_failure_stops_install(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture(root / "source")
            stage(root / "source", root / "output", True)
            destination = root / "AOS.app"
            destination.mkdir()
            manager = root / "output/macos/manage-macos-fskit.sh"
            manager.write_text('#!/bin/sh\n[ "$1" != validate ] || exit 19\nexit 0\n')
            result = subprocess.run(
                ["/bin/sh", str(root / "output/macos/aos-filesystem.sh"), "install"],
                env={**os.environ, "ASTRID_FSKIT_APP_DEST": str(destination)},
                check=False,
            )
            self.assertEqual(result.returncode, 19)

    def test_rejects_missing_signature_and_links(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture(root / "source")
            signature = root / "source/AstridFS.app/Contents/_CodeSignature/CodeResources"
            signature.unlink()
            with self.assertRaises(ValueError):
                stage(root / "source", root / "output", True)
            signature.symlink_to(root / "outside")
            with self.assertRaises(ValueError):
                stage(root / "source", root / "output", True)


if __name__ == "__main__":
    unittest.main()
