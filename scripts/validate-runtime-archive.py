#!/usr/bin/env python3
"""Validate a pinned Astrid runtime archive before extracting required tools."""

from __future__ import annotations

import argparse
import sys
import tarfile
import unicodedata
from pathlib import Path, PurePosixPath


class ArchiveError(RuntimeError):
    """The runtime archive is ambiguous or unsafe to extract."""


def canonical_name(member: tarfile.TarInfo, archive: Path) -> str:
    name = member.name
    raw = name[:-1] if member.isdir() and name.endswith("/") else name
    parts = raw.split("/")
    path = PurePosixPath(raw)
    if (
        not raw
        or path.is_absolute()
        or any(part in ("", ".", "..") for part in parts)
        or path.as_posix() != raw
    ):
        raise ArchiveError(f"{archive}: unsafe or non-canonical path {name!r}")
    return raw


def validate(archive_path: Path, root: str, tools: list[str]) -> None:
    if not archive_path.is_file() or archive_path.is_symlink():
        raise ArchiveError(f"runtime archive is missing or not a regular file: {archive_path}")
    if not root or "/" in root or root in (".", ".."):
        raise ArchiveError(f"invalid runtime archive root: {root!r}")
    if not tools:
        raise ArchiveError("at least one runtime tool is required")
    for tool in tools:
        if not tool or "/" in tool or tool in (".", ".."):
            raise ArchiveError(f"invalid runtime tool: {tool!r}")

    try:
        archive = tarfile.open(archive_path, mode="r:gz")
    except (OSError, tarfile.TarError) as error:
        raise ArchiveError(f"cannot read runtime archive: {error}") from error

    names: dict[str, tarfile.TarInfo] = {}
    portable_names: dict[str, str] = {}
    with archive:
        for member in archive.getmembers():
            if not (member.isfile() or member.isdir()):
                raise ArchiveError(f"runtime archive contains a link or special file: {member.name}")
            name = canonical_name(member, archive_path)
            if name != root and not name.startswith(f"{root}/"):
                raise ArchiveError(f"runtime archive path is outside {root}: {name}")
            if name in names:
                raise ArchiveError(f"runtime archive contains duplicate path: {name}")
            portable = unicodedata.normalize("NFC", name).casefold()
            if portable in portable_names:
                raise ArchiveError(
                    "runtime archive paths collide on portable filesystems: "
                    f"{portable_names[portable]!r} and {name!r}"
                )
            names[name] = member
            portable_names[portable] = name

    for tool in tools:
        expected = f"{root}/{tool}"
        tool_member = names.get(expected)
        if tool_member is None or not tool_member.isfile() or tool_member.mode & 0o111 == 0:
            raise ArchiveError(f"runtime archive has no executable {expected}")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("archive", type=Path)
    parser.add_argument("root")
    parser.add_argument("tool", nargs="+")
    args = parser.parse_args(argv)
    try:
        validate(args.archive, args.root, args.tool)
    except ArchiveError as error:
        print(f"runtime archive validation error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
