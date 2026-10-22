#!/usr/bin/env python3
"""Fail-closed validation for the private Linux Realm capsule candidate."""

from __future__ import annotations

import argparse
import base64
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
    PROVENANCE_ALGORITHM,
    PROVENANCE_MEMBER,
    PROVENANCE_SCHEMA_VERSION,
    ROOT,
    blake3_file,
    blake3_bytes,
    content_tree_digest,
    declared_members,
    locked_builder_metadata,
    read_source_sha_section,
    read_pinned_source_member,
    archive_members,
    validate_archive_member_name,
    sha256_file,
    verify_builder_provenance,
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
    with path.open("rb") as stream:
        header = stream.read(10)
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
                validate_archive_member_name(info.name)
                if not info.isreg():
                    raise LineageError(f"non-regular capsule member {info.name}")
                if info.pax_headers:
                    raise LineageError(f"capsule member {info.name} has PAX metadata")
                if info.name in members:
                    raise LineageError(f"duplicate capsule member {info.name}")
                if info.name == PROVENANCE_MEMBER and info.size > 64 * 1024:
                    raise LineageError("canonical provenance envelope exceeds 64 KiB")
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


def _hex_field(value: object, length: int, label: str) -> None:
    if (
        not isinstance(value, str)
        or len(value) != length * 2
        or any(character not in "0123456789abcdef" for character in value)
    ):
        raise LineageError(f"lineage {label} must be {length * 2} lowercase hex characters")


def _base64_field(value: object, length: int, label: str) -> None:
    if not isinstance(value, str):
        raise LineageError(f"lineage {label} must be base64 text")
    try:
        decoded = base64.b64decode(value, validate=True)
    except (ValueError, base64.binascii.Error) as error:
        raise LineageError(f"lineage {label} is not canonical base64") from error
    if len(decoded) != length:
        raise LineageError(f"lineage {label} must decode to {length} bytes")


def validate_provenance(
    lineage: dict,
    members: dict[str, bytes],
    builder_path: Path,
    expected_builder_signer: str,
    expected_canonical_signer: str | None,
) -> None:
    builder = lineage.get("builder_provenance")
    if not isinstance(builder, dict):
        raise LineageError("lineage is missing verified builder provenance")
    if (
        builder.get("schema_version") != PROVENANCE_SCHEMA_VERSION
        or builder.get("algorithm") != PROVENANCE_ALGORITHM
    ):
        raise LineageError("lineage builder provenance schema is unsupported")
    _hex_field(builder.get("content_digest"), 32, "builder content digest")
    _base64_field(builder.get("signer"), 32, "builder signer")
    _base64_field(builder.get("signature"), 64, "builder signature")
    recorded_builder_archive = lineage.get("builder_archive")
    if not isinstance(recorded_builder_archive, dict) or (
        recorded_builder_archive.get("blake3") != blake3_file(builder_path)
        or recorded_builder_archive.get("sha256") != sha256_file(builder_path)
    ):
        raise LineageError("builder archive differs from lineage")
    actual_builder = verify_builder_provenance(
        archive_members(builder_path), expected_signer=expected_builder_signer
    )
    if actual_builder != builder:
        raise LineageError("builder provenance differs from the verified builder archive")

    canonical = lineage.get("canonical_provenance")
    if not isinstance(canonical, dict):
        raise LineageError("lineage is missing canonical provenance state")
    state = canonical.get("state")
    content = {
        name: data for name, data in members.items() if name != PROVENANCE_MEMBER
    }
    unsigned_digest = lineage.get("unsigned_content_digest")
    if not isinstance(unsigned_digest, str):
        raise LineageError("lineage is missing the unsigned content digest")
    if content_tree_digest(content) != unsigned_digest:
        raise LineageError("capsule content differs from the unsigned lineage digest")

    if state == "unsigned":
        if PROVENANCE_MEMBER in members:
            raise LineageError("unsigned canonical capsule unexpectedly contains provenance")
        return
    if state != "signed":
        raise LineageError("unsupported canonical provenance state")
    if PROVENANCE_MEMBER not in members:
        raise LineageError("signed canonical capsule is missing its provenance envelope")
    if expected_canonical_signer is None:
        raise LineageError("signed canonical capsule needs a trusted signer pin")
    tar_members = {
        name: (None, data) for name, data in members.items()
    }
    verified = verify_builder_provenance(
        tar_members, label="canonical", expected_signer=expected_canonical_signer
    )
    if canonical != {"state": "signed", **verified}:
        raise LineageError("canonical provenance differs from the lineage record")
    if verified["content_digest"] != unsigned_digest:
        raise LineageError("signed provenance does not cover the recorded canonical tree")


def validate(
    artifact: Path,
    lineage_path: Path,
    expected_source_sha: str,
    builder_path: Path,
    expected_builder_signer: str,
    expected_canonical_signer: str | None = None,
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
    if lineage.get("builder") != locked_builder_metadata():
        raise LineageError("lineage builder OCI or toolchain pins differ from source locks")

    recorded = lineage.get("capsule", {})
    actual_sha256 = sha256_file(artifact)
    actual_blake3 = blake3_file(artifact)
    if (
        artifact.stat().st_size != recorded.get("bytes")
        or actual_sha256 != recorded.get("sha256")
        or actual_blake3 != recorded.get("blake3")
    ):
        raise LineageError("capsule digest differs from private lineage manifest")

    members = read_members(artifact)
    validate_provenance(
        lineage, members, builder_path, expected_builder_signer, expected_canonical_signer
    )
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

    if members["Capsule.toml"] != read_pinned_source_member(MANIFEST_PATH, ROOT):
        raise LineageError("embedded Capsule.toml differs from source manifest")
    if members[LOCK_PATH.relative_to(ROOT).as_posix()] != read_pinned_source_member(LOCK_PATH, ROOT):
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
        if read_pinned_source_member(source, ROOT) != members[path]:
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
    parser.add_argument("--builder", required=True, type=Path)
    parser.add_argument("--builder-signer", required=True)
    parser.add_argument("--canonical-signer")
    args = parser.parse_args()

    artifact = args.artifact.resolve()
    lineage_path = args.lineage.resolve()
    try:
        source_sha = args.source_sha or current_source_sha()
        result = validate(
            artifact,
            lineage_path,
            source_sha,
            args.builder.resolve(),
            args.builder_signer,
            args.canonical_signer,
        )
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
