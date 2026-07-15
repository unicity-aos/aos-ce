#!/usr/bin/env python3
"""Adversarial tests for the frozen Astrid 0.9.4 structural fixture."""

from __future__ import annotations

import importlib.util
import os
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("create-astrid-094-fixture.py")
SPEC = importlib.util.spec_from_file_location("create_astrid_094_fixture", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load frozen fixture generator")
FIXTURE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FIXTURE)


class FrozenFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.root = Path(self.temporary_directory.name) / "astrid-home"
        FIXTURE.create(self.root)

    def assert_invalid(self, pattern: str) -> None:
        with self.assertRaisesRegex(ValueError, pattern):
            FIXTURE.validate(self.root)

    def test_exact_frozen_shape_validates(self) -> None:
        FIXTURE.validate(self.root)

    def test_renamed_configuration_is_rejected(self) -> None:
        (self.root / "etc/groups.toml").rename(self.root / "etc/teams.toml")
        self.assert_invalid("file topology differs")

    def test_missing_profile_is_rejected(self) -> None:
        (self.root / "etc/profiles/principal-6.toml").unlink()
        self.assert_invalid("counts differ|file topology differs")

    def test_unexpected_empty_directory_is_rejected(self) -> None:
        (self.root / "etc/future-hooks").mkdir()
        self.assert_invalid("directory topology differs")

    def test_changed_run_name_is_rejected(self) -> None:
        (self.root / "run/system.pid").rename(self.root / "run/daemon.pid")
        self.assert_invalid("file topology differs")

    def test_root_top_level_and_representative_modes_are_rejected(self) -> None:
        for path, mode in [
            (self.root, 0o755),
            (self.root / "secrets", 0o700),
            (self.root / "keys/runtime.key", 0o600),
            (self.root / "home/alice/.local/state/item-000", 0o644),
        ]:
            with self.subTest(path=path):
                FIXTURE.create(self._reset_root())
                os.chmod(path, mode)
                self.assert_invalid("mode")

    def _reset_root(self) -> Path:
        for path in sorted(self.root.rglob("*"), reverse=True):
            if path.is_symlink() or path.is_file():
                path.unlink()
            else:
                path.rmdir()
        self.root.rmdir()
        return self.root

    @unittest.skipUnless(hasattr(os, "symlink"), "symlinks are unavailable")
    def test_symlink_is_rejected(self) -> None:
        target = self.root.parent / "outside"
        target.write_text("outside\n", encoding="utf-8")
        path = self.root / "keys/runtime.key"
        path.unlink()
        path.symlink_to(target)
        self.assert_invalid("symlink")

    @unittest.skipUnless(hasattr(os, "mkfifo"), "FIFOs are unavailable")
    def test_special_file_is_rejected(self) -> None:
        os.mkfifo(self.root / "var/special")
        self.assert_invalid("special file")

    def test_managed_runtime_executable_is_rejected(self) -> None:
        (self.root / "bin/astrid").write_text("not part of the frozen home\n", encoding="utf-8")
        self.assert_invalid("counts differ|file topology differs")


if __name__ == "__main__":
    unittest.main()
