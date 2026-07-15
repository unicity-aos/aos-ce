#!/usr/bin/env python3
"""Create a sanitized copy of the frozen 2026-07-15 Astrid 0.9.4 home shape."""

from __future__ import annotations

import argparse
import os
import stat
from collections import Counter
from pathlib import Path


EXPECTED_COUNTS = {
    "bin": 63,
    "etc": 9,
    "home": 370,
    "keys": 8,
    "log": 7,
    "run": 5,
    "secrets": 1,
    "var": 9,
    "wit": 11,
}
RUNTIME_EXECUTABLES = {"astrid", "astrid-daemon", "astrid-build", "astrid-emit"}


def write(root: Path, relative: str, content: str) -> None:
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def create(root: Path) -> None:
    if root.exists() and any(root.iterdir()):
        raise ValueError(f"fixture root is not empty: {root}")
    root.mkdir(parents=True, exist_ok=True)

    for index in range(63):
        write(root, f"bin/component-{index:02}.wasm", f"wasm-{index:02}\n")

    write(root, "etc/layout-version", "1\n")
    write(root, "etc/groups.toml", "[groups]\n")
    for index in range(7):
        write(
            root,
            f"etc/profiles/principal-{index}.toml",
            f"enabled = true\nindex = {index}\n",
        )
    (root / "etc/hooks").mkdir(parents=True)

    distro_locks = {
        "alice": ("astralis", "0.2.2"),
        "bob": ("aos-ce", "2026.1.0"),
        "carol": ("unicity-ce", "2026.1.0"),
        "dan": ("other", "1.0.0"),
    }
    for principal, (identifier, version) in distro_locks.items():
        write(
            root,
            f"home/{principal}/.config/distro.lock",
            f'[distro]\nid = "{identifier}"\nversion = "{version}"\n',
        )
    for index in range(140):
        write(
            root,
            f"home/alice/.local/capsules/asset-{index:03}",
            f"capsule-payload-or-meta-{index:03}\n",
        )
    for index in range(4):
        write(
            root,
            f"home/alice/.local/audit/record-{index:03}",
            f"audit-{index:03}\n",
        )
    for index in range(26):
        write(
            root,
            f"home/alice/.config/env/override-{index:03}",
            f"ENV_{index:03}=preserved\n",
        )
    for index in range(196):
        write(
            root,
            f"home/alice/.local/state/item-{index:03}",
            f"principal-state-{index:03}\n",
        )

    write(root, "keys/runtime.key", "synthetic-runtime-key-material\n")
    for index in range(7):
        write(root, f"keys/device-{index}.key", f"synthetic-device-key-{index}\n")
    for index in range(7):
        write(root, f"log/runtime-{index}.log", f"log-{index}\n")
    for relative, content in {
        ".hud-health": "stale HUD health\n",
        "session.principal": "transient-principal\n",
        "system.lock": "daemon-lock\n",
        "system.pid": "12345\n",
        "system.token": "ephemeral-credential\n",
    }.items():
        write(root, f"run/{relative}", content)
    write(root, "secrets/providers.toml", 'token = "synthetic-secret"\n')
    for index in range(9):
        write(root, f"var/state-{index}", f"state-{index}\n")
    for index in range(11):
        write(
            root,
            f"wit/contract-{index}.wit",
            f"package fixture:contract{index};\n",
        )

    for directory in EXPECTED_COUNTS:
        os.chmod(root / directory, 0o700)
    os.chmod(root, 0o700)
    os.chmod(root / "secrets", 0o755)
    os.chmod(root / "home/alice/.local/state/item-000", 0o755)
    validate(root)


def validate(root: Path) -> None:
    counts: Counter[str] = Counter()
    files: list[Path] = []
    for path in root.rglob("*"):
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            raise ValueError(f"fixture contains a symlink: {path}")
        if path.is_file():
            relative = path.relative_to(root)
            counts[relative.parts[0]] += 1
            files.append(relative)
        elif not path.is_dir():
            raise ValueError(f"fixture contains a special file: {path}")

    if dict(counts) != EXPECTED_COUNTS:
        raise ValueError(f"fixture counts differ: expected {EXPECTED_COUNTS}, found {dict(counts)}")
    if len(files) != 483:
        raise ValueError(f"fixture must contain 483 regular files, found {len(files)}")

    bin_entries = [path for path in files if path.parts[0] == "bin"]
    if not all(path.suffix == ".wasm" for path in bin_entries):
        raise ValueError("every frozen bin entry must be a WASM component")
    if any((root / "bin" / name).exists() for name in RUNTIME_EXECUTABLES):
        raise ValueError("the frozen home must not contain managed runtime executables")
    hooks = root / "etc/hooks"
    if not hooks.is_dir() or any(hooks.iterdir()):
        raise ValueError("the frozen etc/hooks directory must be empty")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    args = parser.parse_args()
    create(args.root.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
