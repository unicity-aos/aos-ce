#!/usr/bin/env python3
"""Carry the upstream signed filesystem bundle without modifying its contents."""

from pathlib import Path
import shutil
import sys


def stage(source: Path, destination: Path, required: bool) -> None:
    app = source / "AstridFS.app"
    if not app.exists() and not app.is_symlink():
        if required:
            raise ValueError("Darwin runtime archive is missing AstridFS.app")
        return
    if not app.is_dir() or app.is_symlink():
        raise ValueError("AstridFS.app must be a regular directory")
    scripts = [source / "macos" / name for name in (
        "manage-macos-fskit.sh", "validate-macos-fskit.sh",
    )]
    required_members = [app / name for name in (
        "Contents/Info.plist", "Contents/MacOS/AstridFS",
        "Contents/_CodeSignature/CodeResources",
        "Contents/Extensions/AstridFSAppEx.appex/Contents/Info.plist",
        "Contents/Extensions/AstridFSAppEx.appex/Contents/MacOS/AstridFSAppEx",
        "Contents/Extensions/AstridFSAppEx.appex/Contents/_CodeSignature/CodeResources",
    )]
    for member in [*required_members, *scripts]:
        if not member.is_file() or member.is_symlink():
            raise ValueError(f"missing filesystem bundle member: {member}")
    for member in app.rglob("*"):
        if member.is_symlink() or not (member.is_dir() or member.is_file()):
            raise ValueError(f"unsupported filesystem bundle member: {member}")
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copytree(app, destination / "AstridFS.app", copy_function=shutil.copy2)
    (destination / "macos").mkdir()
    for script in scripts:
        shutil.copy2(script, destination / "macos" / script.name)
    shutil.copy2(Path(__file__).with_name("aos-filesystem.sh"),
                 destination / "macos" / "aos-filesystem.sh")


if __name__ == "__main__":
    stage(Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3] == "required")
