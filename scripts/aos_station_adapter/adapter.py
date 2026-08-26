"""Fail-closed AOS Station v2 preparation and deterministic local output."""

from __future__ import annotations

import hashlib
import io
import json
import os
import stat
import subprocess
import tarfile
import tempfile
from pathlib import Path
from pathlib import PurePosixPath
from typing import Any

from . import policy


class AdapterError(RuntimeError):
    """The adapter cannot safely produce the requested local submission."""


class InertOnlyError(AdapterError):
    """The caller did not explicitly select the non-authoritative fixture path."""


BRIDGE_SOURCE = Path(__file__).with_name("prepare_v2_bridge.rs")


def _git_command(*arguments: str) -> list[str]:
    """Run Git inspection/export commands without replacement refs."""
    return ["git", "--no-replace-objects", *arguments]


def _canonical_json(value: Any) -> bytes:
    return (
        json.dumps(value, indent=2, sort_keys=True, ensure_ascii=True) + "\n"
    ).encode("utf-8")


def _write_regular(path: Path, content: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.is_symlink():
        raise AdapterError(f"refusing to overwrite symlink: {path}")
    path.write_bytes(content)


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _tree_manifest(root: Path) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    for path in sorted(item for item in root.rglob("*") if item.is_file()):
        if path.is_symlink():
            raise AdapterError(f"output tree contains a symlink: {path}")
        entries.append(
            {
                "path": path.relative_to(root).as_posix(),
                "sha256": _sha256(path),
                "size": path.stat().st_size,
            }
        )
    return entries


def _run(
    command: list[str],
    *,
    context: str,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    try:
        completed = subprocess.run(
            command,
            check=False,
            capture_output=True,
            text=True,
            env=env,
        )
    except OSError as error:
        raise AdapterError(f"cannot invoke {context}: {error}") from error
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or "unknown error"
        raise AdapterError(f"{context} failed: {detail}")
    return completed


def _run_bytes(command: list[str], *, context: str) -> bytes:
    """Run a command whose output must remain byte-for-byte intact."""
    try:
        completed = subprocess.run(
            command,
            check=False,
            capture_output=True,
        )
    except OSError as error:
        raise AdapterError(f"cannot invoke {context}: {error}") from error
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", "replace").strip() or "unknown error"
        raise AdapterError(f"{context} failed: {detail}")
    return completed.stdout


def _b3sum_bytes(data: bytes, executable: str) -> str:
    with tempfile.NamedTemporaryFile(prefix="aos-station-fixture-", delete=False) as handle:
        path = Path(handle.name)
        handle.write(data)
    try:
        completed = _run([executable, "--", str(path)], context="BLAKE3 fixture digest")
        value = completed.stdout.split()
        if not value or len(value[0]) != 64 or any(char not in "0123456789abcdef" for char in value[0]):
            raise AdapterError("BLAKE3 fixture digest tool returned malformed output")
        return value[0]
    finally:
        path.unlink(missing_ok=True)


def _source_tree_digest(source_root: Path, executable: str) -> str:
    """Hash the exact tagged tree listing used for source provenance."""
    completed = _run(
        _git_command(
            "-C",
            str(source_root),
            "ls-tree",
            "-r",
            "-l",
            policy.SOURCE_COMMIT,
        ),
        context="tagged source tree listing",
    )
    return _b3sum_bytes(completed.stdout.encode("utf-8"), executable)


def _git_tree(station_tools: Path) -> dict[str, tuple[str, int, int]]:
    """Read the expected path, object, and mode directly from the pinned tree."""
    listing = _run(
        _git_command(
            "-C",
            str(station_tools),
            "ls-tree",
            "-r",
            "-z",
            "--full-tree",
            "--long",
            policy.STATION_TOOLS_COMMIT,
        ),
        context="station-tools pinned tree listing",
    ).stdout
    expected: dict[str, tuple[str, int, int]] = {}
    for record in listing.rstrip("\0").split("\0"):
        if not record:
            continue
        metadata, path = record.split("\t", 1)
        mode, object_type, object_id, size = metadata.split(" ", 3)
        if object_type != "blob" or mode not in {"100644", "100755"}:
            raise AdapterError(
                "station-tools pinned tree contains an unexpected non-regular entry: "
                f"{mode} {object_type} {path}"
            )
        if path == ".git" or path.startswith(".git/"):
            raise AdapterError(f"station-tools pinned tree contains reserved path: {path}")
        expected[path] = (object_id, int(size), int(mode, 8) & 0o777)
    if not expected:
        raise AdapterError("station-tools pinned tree is empty")
    return expected


def _index_flags(station_tools: Path) -> None:
    """Reject index optimization flags without relying on index dirt checks."""
    listing = _run(
        _git_command("-C", str(station_tools), "ls-files", "-v", "-z"),
        context="station-tools index flag check",
    ).stdout
    for record in listing.rstrip("\0").split("\0"):
        if not record:
            continue
        marker, separator, path = record.partition(" ")
        if not separator:
            raise AdapterError("station-tools index listing is malformed")
        if marker.islower() or marker == "S":
            raise AdapterError(
                "station-tools index has assume-unchanged or skip-worktree flag: "
                f"{path}"
            )


def _git_blob_digest(data: bytes) -> str:
    header = f"blob {len(data)}\0".encode("ascii")
    return hashlib.sha1(header + data).hexdigest()


def _verify_worktree_tree(station_tools: Path, expected: dict[str, tuple[str, int, int]]) -> None:
    """Compare every worktree byte and mode to Git objects, including ignored paths."""
    expected_files = set(expected)
    expected_dirs = {""}
    for path in expected_files:
        parts = path.split("/")
        expected_dirs.update("/".join(parts[:index]) for index in range(1, len(parts)))

    def check_entry(path: Path, relative: str) -> None:
        try:
            metadata = path.lstat()
        except OSError as error:
            raise AdapterError(f"cannot inspect station-tools entry {path}: {error}") from error
        if stat.S_ISLNK(metadata.st_mode):
            raise AdapterError(f"station-tools checkout contains an unexpected symlink: {relative}")
        if stat.S_ISDIR(metadata.st_mode):
            if relative not in expected_dirs:
                raise AdapterError(f"station-tools checkout has an extra filesystem entry: {relative}")
            try:
                entries = list(path.iterdir())
            except OSError as error:
                raise AdapterError(f"cannot read station-tools directory {path}: {error}") from error
            for child in entries:
                child_relative = child.name if not relative else f"{relative}/{child.name}"
                if relative or child.name != ".git":
                    check_entry(child, child_relative)
            return
        if not stat.S_ISREG(metadata.st_mode):
            raise AdapterError(f"station-tools checkout has an unexpected filesystem entry: {relative}")
        if relative not in expected_files:
            raise AdapterError(f"station-tools checkout has an extra filesystem entry: {relative}")
        object_id, expected_size, expected_mode = expected[relative]
        mode = stat.S_IMODE(metadata.st_mode)
        if mode != expected_mode:
            raise AdapterError(
                f"station-tools mode drift for {relative}: expected {expected_mode:o}, got {mode:o}"
            )
        try:
            content = path.read_bytes()
        except OSError as error:
            raise AdapterError(f"cannot read station-tools entry {path}: {error}") from error
        if len(content) != expected_size or _git_blob_digest(content) != object_id:
            raise AdapterError(f"station-tools content drift for {relative}")

    if station_tools.is_symlink() or not station_tools.is_dir():
        raise AdapterError(f"station-tools checkout is not a directory: {station_tools}")
    for child in station_tools.iterdir():
        if child.name == ".git":
            if child.is_symlink():
                raise AdapterError("station-tools checkout contains an unexpected .git symlink")
            continue
        check_entry(child, child.name)
    actual_files: set[str] = set()
    actual_dirs: set[str] = {""}
    for path in station_tools.rglob("*"):
        relative = path.relative_to(station_tools).as_posix()
        if relative == ".git" or relative.startswith(".git/"):
            continue
        if path.is_symlink():
            raise AdapterError(f"station-tools checkout contains an unexpected symlink: {relative}")
        if path.is_dir():
            actual_dirs.add(relative)
        elif path.is_file():
            actual_files.add(relative)
    if actual_files != expected_files or actual_dirs != expected_dirs:
        missing = sorted(expected_files - actual_files)
        extras = sorted(actual_files - expected_files)
        missing_dirs = sorted(expected_dirs - actual_dirs)
        extra_dirs = sorted(actual_dirs - expected_dirs)
        detail = ", ".join(
            item
            for item in (
                f"missing={missing[:3]}" if missing else "",
                f"extra={extras[:3]}" if extras else "",
                f"missing_dirs={missing_dirs[:3]}" if missing_dirs else "",
                f"extra_dirs={extra_dirs[:3]}" if extra_dirs else "",
            )
            if item
        )
        raise AdapterError(f"station-tools filesystem tree differs from pinned objects: {detail}")


def check_station_tools_pin(station_tools: Path) -> dict[str, str]:
    """Require the exact reviewed local v2 interface and immutable tree."""
    if not station_tools.is_dir() or station_tools.is_symlink():
        raise AdapterError(f"station-tools checkout is not a directory: {station_tools}")
    head = _run(
        _git_command("-C", str(station_tools), "rev-parse", "HEAD^{commit}"),
        context="station-tools pin check",
    ).stdout.strip()
    tree = _run(
        _git_command("-C", str(station_tools), "rev-parse", "HEAD^{tree}"),
        context="station-tools tree check",
    ).stdout.strip()
    if head != policy.STATION_TOOLS_COMMIT or tree != policy.STATION_TOOLS_TREE:
        raise AdapterError(
            "station-tools checkout is not the reviewed v2 interface "
            f"(head={head!r}, tree={tree!r})"
        )
    _index_flags(station_tools)
    expected = _git_tree(station_tools)
    _verify_worktree_tree(station_tools, expected)
    dirt = _run(
        _git_command(
            "-C",
            str(station_tools),
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
        ),
        context="station-tools dirt check",
    ).stdout
    if dirt.strip():
        raise AdapterError(
            "station-tools checkout is dirty; must not compile prepare_v2 from a dirty tree"
        )
    manifest = station_tools / "Cargo.toml"
    if not manifest.is_file() or manifest.is_symlink():
        raise AdapterError(f"station-tools Cargo.toml is missing: {manifest}")
    return {
        "repository": "astrid-runtime/station-tools",
        "commit": head,
        "tree": tree,
        "interface": "astrid-station-publish::prepare_v2",
    }


def emit_submission_skeleton(root: Path, verification: dict[str, Any]) -> dict[str, Any]:
    """Create the typed submission roots and a non-authoritative proposal."""
    if root.exists():
        if root.is_symlink() or not root.is_dir():
            raise AdapterError(f"output path is not a directory: {root}")
        if any(root.iterdir()):
            raise AdapterError(f"output path must be fresh or empty: {root}")
    root.mkdir(parents=True, exist_ok=True)
    submission = root / "submission"
    reports = root / "reports"
    for directory in (submission, reports):
        directory.mkdir(parents=True, exist_ok=True)

    proposals = {
        "schema": "aos-station-v2-proposal-v1",
        "readiness": False,
        "blockers": ["admission-contract-pending", "owner-binding-unresolved"],
        "station_id": policy.STATION_ID,
        "source": {
            "repository": policy.SOURCE_REPOSITORY,
            "commit": policy.SOURCE_COMMIT,
            "tree": policy.SOURCE_TREE,
            "tag": policy.SOURCE_TAG,
            "workflow_identity": policy.SOURCE_WORKFLOW_IDENTITY,
        },
        "allowlist_coordinates": [item.coordinate for item in policy.SEED],
        "allowlist_directories": [item.directory for item in policy.SEED],
        "blocked": [
            {
                "directory": directory,
                "package": package,
                "reason": "current-main extra; wait for a new signed versioned AOS release",
            }
            for directory, package in sorted(policy.BLOCKED_DIRECTORIES.items())
        ],
        "excluded": [
            {"package": "aos-openai", "reason": "not in the 2026.1.3 community seed"},
            {"package": "aos-telegram", "reason": "not in the 2026.1.3 community seed"},
            {
                "package": "astrid-capsule-*",
                "reason": "legacy standalone identity; never an AOS CE seed coordinate",
            },
        ],
    }
    _write_regular(
        submission / "proposals" / "aos-ce-2026.1.3.json", _canonical_json(proposals)
    )
    _write_regular(
        submission / "README.md",
        (
            "# AOS CE Station v2 proposal\n\n"
            "This local tree is a readiness fixture. It contains sealed v2 "
            "publication records produced by the reviewed `prepare_v2` API, "
            "but readiness remains false: fixture publisher material is not a "
            "namespace owner or live signing authority. This adapter cannot "
            "activate, push, sign, or publish Station content.\n"
        ).encode("utf-8"),
    )
    _write_regular(reports / "release-verification.json", _canonical_json(verification))
    for directory in ("records", "events", "namespaces", "artifacts"):
        (submission / directory).mkdir(parents=True, exist_ok=True)
    return {
        "submission": submission,
        "reports": reports,
        "proposal_count": len(policy.SEED),
        "record_count": 0,
        "event_count": 0,
    }


def _materialize_station_tools(station_tools: Path) -> Path:
    """Export only pinned Git objects into a fresh tree and verify every byte."""
    expected = _git_tree(station_tools)
    archive = _run_bytes(
        _git_command(
            "-C",
            str(station_tools),
            "archive",
            "--format=tar",
            "--prefix=",
            policy.STATION_TOOLS_COMMIT,
        ),
        context="station-tools immutable Git export",
    )
    export = Path(tempfile.mkdtemp(prefix="aos-station-tools-export-"))
    seen_files: set[str] = set()
    seen_dirs: set[str] = {""}
    try:
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as reader:
            for member in reader:
                name = member.name.rstrip("/")
                path = PurePosixPath(name)
                if (
                    not name
                    or path.is_absolute()
                    or ".." in path.parts
                    or "." in path.parts
                    or "\\" in name
                ):
                    raise AdapterError(f"station-tools Git export contains an unsafe path: {member.name}")
                relative = path.as_posix()
                if member.isdir():
                    seen_dirs.add(relative)
                    (export / relative).mkdir(parents=True, exist_ok=True)
                    continue
                if member.issym() or member.islnk() or not member.isfile():
                    raise AdapterError(f"station-tools Git export contains an unexpected symlink or entry: {relative}")
                if relative not in expected:
                    raise AdapterError(f"station-tools Git export contains an unexpected path: {relative}")
                stream = reader.extractfile(member)
                if stream is None:
                    raise AdapterError(f"station-tools Git export has no content for {relative}")
                content = stream.read()
                object_id, expected_size, expected_mode = expected[relative]
                if (member.mode & 0o111) != (expected_mode & 0o111):
                    raise AdapterError(f"station-tools Git export mode drift for {relative}")
                if len(content) != expected_size or _git_blob_digest(content) != object_id:
                    raise AdapterError(f"station-tools Git export content drift for {relative}")
                destination = export / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                if destination.exists() or destination.is_symlink():
                    raise AdapterError(f"station-tools Git export contains duplicate path: {relative}")
                destination.write_bytes(content)
                seen_files.add(relative)
        expected_dirs = {""}
        for path in expected:
            parts = path.split("/")
            expected_dirs.update("/".join(parts[:index]) for index in range(1, len(parts)))
        if seen_files != set(expected) or seen_dirs != expected_dirs:
            raise AdapterError("station-tools Git export does not match the pinned tree")
    except (OSError, tarfile.TarError) as error:
        raise AdapterError(f"cannot materialize station-tools Git export: {error}") from error
    return export


def _freeze_export(export: Path) -> None:
    """Make source files immutable while leaving Cargo a writable target root."""
    for path in sorted(export.rglob("*"), key=lambda value: len(value.parts), reverse=True):
        if path.is_symlink():
            raise AdapterError(f"immutable station-tools export contains a symlink: {path}")
        try:
            if path.is_dir():
                path.chmod(0o555)
            elif path.is_file():
                path.chmod(0o444)
            else:
                raise AdapterError(f"immutable station-tools export contains an unexpected entry: {path}")
        except OSError as error:
            raise AdapterError(f"cannot freeze station-tools export {path}: {error}") from error


def _bridge_manifest(station_tools: Path, cargo: str) -> Path:
    """Build only from a verified Git export, never from the operator checkout."""
    if not BRIDGE_SOURCE.is_file() or BRIDGE_SOURCE.is_symlink():
        raise AdapterError(f"v2 bridge source is missing: {BRIDGE_SOURCE}")
    # Re-check at the build boundary so direct callers cannot bypass the
    # worktree and index validation performed by prepare_v2_records.
    check_station_tools_pin(station_tools)
    export = _materialize_station_tools(station_tools)
    station_publish = export / "crates" / "station-publish"
    station_protocol = export / "crates" / "station-protocol"
    if not station_publish.is_dir() or not station_protocol.is_dir():
        raise AdapterError("reviewed Station export lacks station-publish/protocol crates")
    bridge_source = station_publish / "src" / "bin" / "aos-station-v2-bridge.rs"
    bridge_source.parent.mkdir(parents=True, exist_ok=True)
    bridge_source.write_bytes(BRIDGE_SOURCE.read_bytes())
    _freeze_export(export)
    build_env = os.environ.copy()
    build_env["CARGO_TARGET_DIR"] = str(export / "target")
    manifest = export / "Cargo.toml"
    _run(
        [cargo, "fetch", "--manifest-path", str(manifest), "--locked"],
        context="station v2 locked dependency fetch",
        env=build_env,
    )
    _run(
        [
            cargo,
            "build",
            "--quiet",
            "--offline",
            "--locked",
            "--manifest-path",
            str(manifest),
            "--package",
            "astrid-station-publish",
            "--bin",
            "aos-station-v2-bridge",
        ],
        context="station v2 bridge build",
        env=build_env,
    )
    binary = export / "target" / "debug" / "aos-station-v2-bridge"
    if not binary.is_file() or binary.is_symlink():
        raise AdapterError("station v2 bridge build did not produce an executable")
    return binary


def _record_summary(value: dict[str, Any], item: policy.SeedPackage) -> dict[str, Any]:
    if value.get("schema") != "aos-station-prepare-v2-result-v1":
        raise AdapterError(f"{item.package}: bridge returned an unknown result schema")
    if value.get("readiness") is not False or value.get("fixture") is not True:
        raise AdapterError(f"{item.package}: bridge result is not the non-admitting fixture")
    required = {
        "record",
        "artifact",
        "coordinate",
        "version",
        "publication_digest",
        "package_digest",
        "manifest_digest",
        "content_digest",
        "artifact_size",
        "classification",
        "readiness",
        "fixture",
        "schema",
    }
    if set(value) != required:
        raise AdapterError(f"{item.package}: bridge returned unknown or missing fields")
    expected_coordinate = f"@{policy.RESERVED_NAMESPACE}/{item.package}"
    if value["coordinate"] != expected_coordinate or value["version"] != item.version:
        raise AdapterError(f"{item.package}: bridge coordinate/version mismatch")
    if value["classification"] != "New":
        raise AdapterError(f"{item.package}: unexpected publication classification")
    return {
        "coordinate": value["coordinate"],
        "version": value["version"],
        "record": value["record"],
        "artifact": value["artifact"],
        "publication_digest": value["publication_digest"],
        "package_digest": value["package_digest"],
        "manifest_digest": value["manifest_digest"],
        "content_digest": value["content_digest"],
        "artifact_size": value["artifact_size"],
        "readiness": False,
        "fixture": True,
    }


def prepare_v2_records(
    station_tools: Path,
    artifacts: Path,
    source_root: Path,
    submission: Path,
    *,
    b3sum: str = "b3sum",
    cargo: str = "cargo",
) -> dict[str, Any]:
    """Call `prepare_v2` once per seed package with explicit fixture identity."""
    tool = check_station_tools_pin(station_tools)
    bridge = _bridge_manifest(station_tools, cargo)
    source_digest = _source_tree_digest(source_root, b3sum)
    statement_digest = _b3sum_bytes(b"aos-ce-station-v2-readiness-fixture", b3sum)
    publisher_digest = _b3sum_bytes(b"aos-ce-station-v2-fixture-publisher", b3sum)
    summaries: list[dict[str, Any]] = []
    for item in policy.SEED:
        capsule = artifacts / item.asset
        if not capsule.is_file() or capsule.is_symlink():
            raise AdapterError(f"missing capsule artifact for {item.package}: {capsule}")
        manifest = source_root / "capsules" / item.directory / "Capsule.toml"
        try:
            import tomllib

            with manifest.open("rb") as handle:
                tagged_manifest = tomllib.load(handle)
                package_table = tagged_manifest.get("package", {})
        except (OSError, ValueError) as error:
            raise AdapterError(f"cannot read runtime requirement from {manifest}: {error}") from error
        tagged_name = package_table.get("name")
        tagged_version = package_table.get("version")
        if tagged_name != item.package or tagged_version != item.version:
            raise AdapterError(
                f"{item.directory}: tagged Capsule.toml identity does not match the 19-entry allowlist"
            )
        coordinate = f"@{policy.RESERVED_NAMESPACE}/{tagged_name}"
        runtime = package_table.get("astrid-version", "*")
        if not isinstance(runtime, str) or not runtime:
            raise AdapterError(f"{item.package}: invalid package.astrid-version")
        command = [
            str(bridge),
            "--artifact",
            str(capsule),
            "--output",
            str(submission),
            "--station-id",
            policy.STATION_ID,
            "--station-base",
            policy.STATION_BASE_URL,
            "--coordinate",
            coordinate,
            "--version",
            tagged_version,
            "--publisher",
            "fixture:aos-ce-readiness-v2",
            "--publisher-digest",
            f"blake3:{publisher_digest}",
            "--source-repository",
            policy.SOURCE_REPOSITORY,
            "--github-owner-id",
            str(policy.SOURCE_OWNER_ID),
            "--github-repository-id",
            str(policy.SOURCE_REPOSITORY_ID),
            "--source-commit",
            policy.SOURCE_COMMIT,
            "--source-tree",
            policy.SOURCE_TREE,
            "--source-tag",
            policy.SOURCE_TAG,
            "--source-digest",
            f"blake3:{source_digest}",
            "--predicate-type",
            "https://slsa.dev/provenance/v1",
            "--statement-digest",
            f"blake3:{statement_digest}",
            "--builder-identity",
            "https://example.invalid/aos-ce-readiness-fixture",
            "--attestation-identity",
            "fixture:aos-ce-readiness-v2",
            "--runtime",
            runtime,
            "--abi",
            "component-model-v1",
        ]
        completed = _run(command, context=f"prepare_v2 for {item.coordinate}")
        try:
            value = json.loads(completed.stdout)
        except json.JSONDecodeError as error:
            raise AdapterError(f"{item.package}: prepare_v2 bridge returned non-JSON output") from error
        if not isinstance(value, dict):
            raise AdapterError(f"{item.package}: prepare_v2 bridge result is not an object")
        summaries.append(_record_summary(value, item))
    if len(summaries) != len(policy.SEED):
        raise AdapterError("prepare_v2 did not produce exactly the canonical 19 records")
    records = sorted(summaries, key=lambda value: value["coordinate"])
    return {
        "tool": tool,
        "interface": "astrid-station-publish::prepare_v2",
        "fixture": {
            "readiness": False,
            "publisher": "fixture:aos-ce-readiness-v2",
            "binding": "non-admitting; structural fixture only",
        },
        "records": records,
        "record_count": len(records),
        "event_count": 0,
    }
