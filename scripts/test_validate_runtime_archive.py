#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import io
import tarfile
import tempfile
import unittest
from pathlib import Path
from typing import Optional


SCRIPT = Path(__file__).resolve().parent / "validate-runtime-archive.py"
SPEC = importlib.util.spec_from_file_location("validate_runtime_archive", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

ArchiveError = MODULE.ArchiveError
validate = MODULE.validate


def add_member(
    archive: tarfile.TarFile,
    name: str,
    *,
    mode: int = 0o755,
    kind: Optional[bytes] = None,
    data: bytes = b"tool",
) -> None:
    member = tarfile.TarInfo(name)
    member.mode = mode
    member.size = len(data)
    if kind is not None:
        member.type = kind
        member.size = 0
    archive.addfile(member, io.BytesIO(data) if member.isfile() else None)


class RuntimeArchiveTests(unittest.TestCase):
    root = "astrid-0.9.4-x86_64-unknown-linux-gnu"
    tool = "astrid-build"

    def fixture(self, directory: Path, mutation: Optional[str] = None) -> Path:
        path = directory / "runtime.tar.gz"
        with tarfile.open(path, mode="w:gz") as archive:
            add_member(archive, self.root, kind=tarfile.DIRTYPE)
            add_member(archive, f"{self.root}/{self.tool}")
            if mutation == "duplicate":
                add_member(archive, f"{self.root}/{self.tool}", data=b"later")
            if mutation == "dot-alias":
                add_member(archive, f"{self.root}/./{self.tool}", data=b"later")
            if mutation == "case-alias":
                add_member(archive, f"{self.root}/Astrid-Build", data=b"later")
            if mutation == "traversal":
                add_member(archive, f"{self.root}/../escape")
            if mutation == "outside-root":
                add_member(archive, "other/file")
            if mutation == "symlink":
                add_member(archive, f"{self.root}/link", kind=tarfile.SYMTYPE)
            if mutation == "hardlink":
                add_member(archive, f"{self.root}/hard", kind=tarfile.LNKTYPE)
            if mutation == "device":
                add_member(archive, f"{self.root}/device", kind=tarfile.CHRTYPE)
        return path

    def test_accepts_canonical_archive(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            validate(self.fixture(Path(temp)), self.root, [self.tool])

    def test_rejects_non_executable_tool(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "runtime.tar.gz"
            with tarfile.open(path, mode="w:gz") as archive:
                add_member(archive, self.root, kind=tarfile.DIRTYPE)
                add_member(archive, f"{self.root}/{self.tool}", mode=0o644)
            with self.assertRaises(ArchiveError):
                validate(path, self.root, [self.tool])

    def assert_rejected(self, mutation: str) -> None:
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(ArchiveError):
                validate(self.fixture(Path(temp), mutation), self.root, [self.tool])

    def test_rejects_duplicate(self) -> None:
        self.assert_rejected("duplicate")

    def test_rejects_dot_alias(self) -> None:
        self.assert_rejected("dot-alias")

    def test_rejects_case_alias(self) -> None:
        self.assert_rejected("case-alias")

    def test_rejects_traversal(self) -> None:
        self.assert_rejected("traversal")

    def test_rejects_outside_root(self) -> None:
        self.assert_rejected("outside-root")

    def test_rejects_symlink(self) -> None:
        self.assert_rejected("symlink")

    def test_rejects_hardlink(self) -> None:
        self.assert_rejected("hardlink")

    def test_rejects_device(self) -> None:
        self.assert_rejected("device")


if __name__ == "__main__":
    unittest.main()
