#!/usr/bin/env python3
"""Carry a prebuilt Command Center bundle without modifying its contents."""

from pathlib import Path
import plistlib
import shutil
import sys

BUNDLE_NAME = "AOS Command Center.app"
BUNDLE_IDENTIFIER = "ai.unicity.aos.tray"
BUNDLE_EXECUTABLE = "aos-tray"


def stage(source: Path, destination: Path) -> None:
    if not source.exists() or source.is_symlink() or not source.is_dir():
        raise ValueError("Command Center app must be a regular directory")
    if not source.name.endswith(".app"):
        raise ValueError("Command Center app path must end with .app")
    required_members = [
        source / "Contents/Info.plist",
        source / "Contents/MacOS" / BUNDLE_EXECUTABLE,
        source / "Contents/_CodeSignature/CodeResources",
    ]
    for member in required_members:
        if not member.is_file() or member.is_symlink():
            raise ValueError(f"missing Command Center bundle member: {member}")
    with required_members[0].open("rb") as handle:
        plist = plistlib.load(handle)
    if plist.get("CFBundleIdentifier") != BUNDLE_IDENTIFIER:
        raise ValueError("Command Center bundle identifier is not ai.unicity.aos.tray")
    if plist.get("CFBundleExecutable") != BUNDLE_EXECUTABLE:
        raise ValueError("Command Center executable is not aos-tray")
    for member in source.rglob("*"):
        if member.is_symlink() or not (member.is_dir() or member.is_file()):
            raise ValueError(f"unsupported Command Center bundle member: {member}")
    destination.mkdir(parents=True, exist_ok=True)
    target = destination / BUNDLE_NAME
    if target.exists() or target.is_symlink():
        raise ValueError("Command Center staging destination already exists")
    shutil.copytree(source, target, copy_function=shutil.copy2)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit(
            "usage: package_macos_command_center.py <source-app> <share-directory>"
        )
    stage(Path(sys.argv[1]), Path(sys.argv[2]))
