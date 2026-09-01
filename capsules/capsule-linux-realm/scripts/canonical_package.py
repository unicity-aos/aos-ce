#!/usr/bin/env python3
"""Repack the pinned builder output into a byte-stable private capsule."""

from __future__ import annotations

import argparse
import gzip
import io
import json
import os
import subprocess
import tarfile
import tempfile
from pathlib import Path

from lineage import (
    LineageError,
    LOCK_PATH,
    MANIFEST_PATH,
    ROOT,
    archive_members,
    blake3_bytes,
    blake3_file,
    declared_members,
    expected_builder_members,
    manifest,
    source_sha_custom_section,
    sha256_bytes,
    sha256_file,
    validate_declared_hashes,
)


def source_sha() -> str:
    override = os.environ.get("AOS_SOURCE_SHA")
    if override is not None:
        if (
            len(override) != 40
            or any(character not in "0123456789abcdef" for character in override)
        ):
            raise LineageError("AOS_SOURCE_SHA is not a lowercase commit SHA")
        return override
    try:
        return subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=ROOT,
            check=True,
            text=True,
            capture_output=True,
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError) as error:
        raise LineageError(f"unable to identify source commit: {error}") from error


def embed_source_sha(wasm: bytes, expected_source_sha: str) -> bytes:
    if len(wasm) < 8 or wasm[:4] != b"\0asm":
        raise LineageError("controller artifact is not WebAssembly")
    section = source_sha_custom_section(expected_source_sha)
    return wasm[:8] + section + wasm[8:]


def package(builder_path: Path, output_path: Path) -> dict:
    spec = manifest()
    builder = archive_members(builder_path)
    expected = expected_builder_members(spec)
    builder_names = set(builder)

    wit_names = {name for name in builder_names if name.startswith("wit/")}
    if not wit_names:
        raise LineageError("pinned builder did not emit capsule WIT")
    expected.update(wit_names)

    unexpected = builder_names - expected
    if unexpected:
        raise LineageError(f"builder emitted unexpected members: {sorted(unexpected)}")

    missing = expected - builder_names
    if missing:
        raise LineageError(f"builder is missing members: {sorted(missing)}")

    content: dict[str, bytes] = {}
    for name in sorted(expected):
        if name == "Capsule.toml":
            content[name] = MANIFEST_PATH.read_bytes()
        elif name == "aos_linux_realm.wasm":
            content[name] = embed_source_sha(builder[name][1], source_sha())
        elif name == LOCK_PATH.relative_to(ROOT).as_posix():
            content[name] = LOCK_PATH.read_bytes()
        elif name.startswith("wit/"):
            content[name] = builder[name][1]
        else:
            source = ROOT / name
            if not source.is_file():
                raise LineageError(f"missing declared source member: {name}")
            content[name] = source.read_bytes()

    validate_declared_hashes(spec, content)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(prefix=".capsule-", dir=output_path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as raw:
            with gzip.GzipFile(
                filename="",
                mode="wb",
                compresslevel=9,
                fileobj=raw,
                mtime=0,
            ) as compressed:
                with tarfile.open(
                    fileobj=compressed,
                    mode="w",
                    format=tarfile.GNU_FORMAT,
                ) as archive:
                    for name in sorted(content):
                        data = content[name]
                        info = tarfile.TarInfo(name)
                        info.size = len(data)
                        info.mode = 0o644
                        info.mtime = 0
                        info.uid = 0
                        info.gid = 0
                        info.uname = "root"
                        info.gname = "root"
                        info.type = tarfile.REGTYPE
                        archive.addfile(info, io.BytesIO(data))
            raw.flush()
            os.fsync(raw.fileno())
        temporary.replace(output_path)
    finally:
        temporary.unlink(missing_ok=True)

    members = {
        name: {
            "bytes": len(data),
            "blake3": blake3_bytes(data),
            "sha256": sha256_bytes(data),
        }
        for name, data in sorted(content.items())
    }
    return {
        "format": "aos-linux-realm-private-lineage-1",
        "source_sha": source_sha(),
        "builder_archive": {
            "blake3": blake3_file(builder_path),
            "sha256": sha256_file(builder_path),
        },
        "capsule": {
            "bytes": output_path.stat().st_size,
            "blake3": blake3_file(output_path),
            "sha256": sha256_file(output_path),
        },
        "members": members,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--builder",
        type=Path,
        default=ROOT / "build-output/aos-linux-realm.capsule",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "dist/aos-linux-realm.capsule",
    )
    args = parser.parse_args()

    try:
        lineage = package(args.builder.resolve(), args.output.resolve())
    except LineageError as error:
        print(f"error: {error}", file=os.sys.stderr)
        return 2

    lineage_path = args.output.with_suffix(args.output.suffix + ".lineage.json")
    lineage_path.write_text(json.dumps(lineage, indent=2, sort_keys=True) + "\n")
    print(
        f"canonical capsule: {args.output} bytes={lineage['capsule']['bytes']} "
        f"blake3={lineage['capsule']['blake3']} sha256={lineage['capsule']['sha256']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
