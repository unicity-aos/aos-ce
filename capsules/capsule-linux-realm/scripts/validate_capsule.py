#!/usr/bin/env python3
"""Fail-closed validation for the private Linux Realm capsule candidate."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import tarfile
import tomllib
from pathlib import Path

from lineage import (
    LineageError,
    LOCK_PATH,
    MANIFEST_PATH,
    ROOT,
    blake3_file,
    blake3_bytes,
    declared_members,
    read_source_sha_section,
)

FORBIDDEN_CONTROLLER_PATHS = (
    b"/Users/",
    b"/home/runner/",
    b"/opt/hostedtoolcache/",
    b"/root/.cargo/",
)


def current_source_sha() -> str:
    try:
        return subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=ROOT,
            check=True,
            text=True,
            capture_output=True,
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError) as error:
        raise LineageError(f"unable to identify current source commit: {error}") from error


def validate_gzip_header(path: Path) -> None:
    header = path.open("rb").read(10)
    if len(header) != 10 or header[:2] != b"\x1f\x8b" or header[2] != 8:
        raise LineageError("capsule is not a gzip file")
    if header[3] != 0:
        raise LineageError("gzip header contains nondeterministic optional fields")
    if header[4:8] != b"\x00\x00\x00\x00":
        raise LineageError("gzip mtime is not SOURCE_DATE_EPOCH=0")


def read_members(path: Path) -> dict[str, bytes]:
    validate_gzip_header(path)
    members: dict[str, bytes] = {}
    names: list[str] = []
    try:
        with tarfile.open(path, mode="r:gz") as archive:
            for info in archive:
                if not info.isreg():
                    raise LineageError(f"non-regular capsule member {info.name}")
                if info.pax_headers:
                    raise LineageError(f"capsule member {info.name} has PAX metadata")
                if info.name in members:
                    raise LineageError(f"duplicate capsule member {info.name}")
                if (
                    info.mode != 0o644
                    or info.uid != 0
                    or info.gid != 0
                    or info.uname != "root"
                    or info.gname != "root"
                    or info.mtime != 0
                ):
                    raise LineageError(f"capsule member {info.name} has unstable metadata")
                members[info.name] = archive.extractfile(info).read()
                names.append(info.name)
    except (OSError, tarfile.TarError) as error:
        raise LineageError(f"invalid capsule archive {path}: {error}") from error
    if names != sorted(names):
        raise LineageError("capsule members are not sorted")
    return members


def validate(
    artifact: Path,
    lineage_path: Path,
    expected_source_sha: str,
) -> dict:
    try:
        lineage = json.loads(lineage_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise LineageError(f"invalid lineage manifest {lineage_path}: {error}") from error

    if lineage.get("format") != "aos-linux-realm-private-lineage-1":
        raise LineageError("unsupported private lineage format")
    if lineage.get("source_sha") != expected_source_sha:
        raise LineageError(
            f"lineage source mismatch: expected {expected_source_sha}, "
            f"got {lineage.get('source_sha')}"
        )

    recorded = lineage.get("capsule", {})
    actual_sha256 = hashlib.sha256(artifact.read_bytes()).hexdigest()
    actual_blake3 = blake3_file(artifact)
    if (
        artifact.stat().st_size != recorded.get("bytes")
        or actual_sha256 != recorded.get("sha256")
        or actual_blake3 != recorded.get("blake3")
    ):
        raise LineageError("capsule digest differs from private lineage manifest")

    members = read_members(artifact)
    expected_members = lineage.get("members", {})
    if set(members) != set(expected_members):
        raise LineageError(
            "capsule member set differs: "
            f"missing={sorted(set(expected_members) - set(members))}, "
            f"extra={sorted(set(members) - set(expected_members))}"
        )
    for name, expected in expected_members.items():
        content = members[name]
        content_sha256 = hashlib.sha256(content).hexdigest()
        content_blake3 = blake3_bytes(content)
        if len(content) != expected.get("bytes") or content_sha256 != expected.get("sha256") or content_blake3 != expected.get("blake3"):
            raise LineageError(f"stale capsule member {name}")

    if members["Capsule.toml"] != MANIFEST_PATH.read_bytes():
        raise LineageError("embedded Capsule.toml differs from source manifest")
    if members[LOCK_PATH.relative_to(ROOT).as_posix()] != LOCK_PATH.read_bytes():
        raise LineageError("embedded linux-vcpu.lock differs from source lock")
    controller = members["aos_linux_realm.wasm"]
    embedded_source_sha = read_source_sha_section(controller)
    if embedded_source_sha != expected_source_sha:
        raise LineageError(
            "controller source mismatch: "
            f"expected {expected_source_sha}, got {embedded_source_sha}"
        )
    leaked_path = next(
        (marker.decode("ascii") for marker in FORBIDDEN_CONTROLLER_PATHS if marker in controller),
        None,
    )
    if leaked_path is not None:
        raise LineageError(f"controller retains host-absolute path prefix {leaked_path}")

    spec = tomllib.loads(members["Capsule.toml"].decode("utf-8"))
    for path in declared_members(spec):
        if path == "aos_linux_realm.wasm":
            continue
        source = ROOT / path
        if not source.is_file() or source.read_bytes() != members[path]:
            raise LineageError(f"capsule member {path} differs from source")

    return {
        "bytes": artifact.stat().st_size,
        "blake3": actual_blake3,
        "sha256": actual_sha256,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("artifact", nargs="?", type=Path, default=ROOT / "dist/aos-linux-realm.capsule")
    parser.add_argument(
        "--lineage",
        type=Path,
        default=ROOT / "dist/aos-linux-realm.capsule.lineage.json",
    )
    parser.add_argument("--source-sha", type=str)
    args = parser.parse_args()

    artifact = args.artifact.resolve()
    lineage_path = args.lineage.resolve()
    try:
        source_sha = args.source_sha or current_source_sha()
        result = validate(artifact, lineage_path, source_sha)
    except LineageError as error:
        print(f"error: {error}", file=os.sys.stderr)
        return 2

    print(
        f"private Linux Realm capsule verified: bytes={result['bytes']} "
        f"blake3={result['blake3']} sha256={result['sha256']} source={source_sha}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
