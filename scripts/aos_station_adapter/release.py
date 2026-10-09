"""Strict local verification for the signed 2026.1.3 AOS release assets."""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
import tarfile
from pathlib import Path, PurePosixPath
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python 3.10 fallback
    import tomli as tomllib

from . import policy
from .adapter import _git_command


class VerificationError(RuntimeError):
    """A release input did not satisfy the immutable adapter contract."""


def _release_metadata_module():
    scripts = Path(__file__).resolve().parents[1]
    if str(scripts) not in sys.path:
        sys.path.insert(0, str(scripts))
    try:
        import release_metadata
    except ImportError as error:  # pragma: no cover - repository corruption
        raise VerificationError(f"cannot load release metadata validator: {error}") from error
    return release_metadata


def _read_toml(path: Path) -> dict[str, Any]:
    try:
        with path.open("rb") as source:
            value = tomllib.load(source)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise VerificationError(f"{path}: invalid TOML: {error}") from error
    if not isinstance(value, dict):
        raise VerificationError(f"{path}: TOML root must be a table")
    return value


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as source:
            while chunk := source.read(1024 * 1024):
                digest.update(chunk)
    except OSError as error:
        raise VerificationError(f"cannot hash {path}: {error}") from error
    return digest.hexdigest()


def _blake3(path: Path, executable: str) -> str:
    try:
        completed = subprocess.run(
            [executable, "--", str(path)],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as error:
        raise VerificationError(f"cannot run {executable} for {path}: {error}") from error
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or "unknown error"
        raise VerificationError(f"{executable} failed for {path}: {detail}")
    fields = completed.stdout.split()
    if not fields or re.fullmatch(r"[0-9a-f]{64}", fields[0]) is None:
        raise VerificationError(f"{executable} returned malformed digest for {path}")
    return fields[0]


def _checksum_manifest(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise VerificationError(f"cannot read {path}: {error}") from error
    for number, line in enumerate(lines, 1):
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9_.+-]+)", line)
        if match is None:
            raise VerificationError(f"{path}:{number}: malformed checksum entry")
        digest, asset = match.groups()
        if asset in values:
            raise VerificationError(f"{path}: duplicate checksum entry for {asset}")
        values[asset] = digest
    return values


def _canonical_member_name(member: tarfile.TarInfo, asset: Path) -> str:
    raw = member.name[:-1] if member.isdir() and member.name.endswith("/") else member.name
    path = PurePosixPath(raw)
    parts = raw.split("/")
    if (
        not raw
        or path.is_absolute()
        or any(part in ("", ".", "..") for part in parts)
        or path.as_posix() != raw
    ):
        raise VerificationError(f"{asset}: unsafe archive path {member.name!r}")
    return raw


def _load_source_manifest(
    source_root: Path, item: policy.SeedPackage
) -> tuple[dict[str, Any], bytes]:
    path = source_root / "capsules" / item.directory / "Capsule.toml"
    if not path.is_file() or path.is_symlink():
        raise VerificationError(f"missing exact source manifest for {item.package}: {path}")
    try:
        manifest_bytes = path.read_bytes()
    except OSError as error:
        raise VerificationError(f"cannot read exact source manifest {path}: {error}") from error
    manifest = _read_toml(path)
    allowed_top_level = {
        "package",
        "component",
        "imports",
        "exports",
        "publish",
        "subscribe",
        "tool",
        "capabilities",
        "env",
        "context_file",
        "command",
        "mcp_server",
        "skill",
        "uplink",
    }
    unknown_top_level = sorted(set(manifest) - allowed_top_level)
    if unknown_top_level:
        raise VerificationError(
            f"{path}: unknown top-level Capsule.toml fields: {unknown_top_level}"
        )

    def reject_forbidden_tables(value: Any, location: str) -> None:
        if isinstance(value, dict):
            for key, nested in value.items():
                if key in {"authority", "registry", "checkpoint", "event-head"}:
                    raise VerificationError(
                        f"{path}: unsupported {key!r} table at {location}"
                    )
                reject_forbidden_tables(nested, f"{location}.{key}")
        elif isinstance(value, list):
            for index, nested in enumerate(value):
                reject_forbidden_tables(nested, f"{location}[{index}]")

    reject_forbidden_tables(manifest, "Capsule.toml")
    package = manifest.get("package")
    if not isinstance(package, dict):
        raise VerificationError(f"{path}: package table is missing")
    if package.get("name") != item.package or package.get("version") != item.version:
        raise VerificationError(
            f"{path}: package identity/version differs from pinned seed "
            f"{item.package} {item.version}"
        )
    return manifest, manifest_bytes


def _verify_capsule(
    path: Path,
    item: policy.SeedPackage,
    source_manifest: dict[str, Any],
    source_manifest_bytes: bytes,
) -> dict[str, Any]:
    try:
        archive = tarfile.open(path, mode="r:gz")
    except (OSError, tarfile.TarError) as error:
        raise VerificationError(f"{path}: invalid capsule archive: {error}") from error
    with archive:
        members = archive.getmembers()
        by_name: dict[str, tarfile.TarInfo] = {}
        for member in members:
            if not (member.isfile() or member.isdir()):
                raise VerificationError(f"{path}: links and special files are forbidden")
            name = _canonical_member_name(member, path)
            if name in by_name:
                raise VerificationError(f"{path}: duplicate archive path {name!r}")
            by_name[name] = member

        manifest_member = by_name.get("Capsule.toml")
        if manifest_member is None or not manifest_member.isfile():
            raise VerificationError(f"{path}: Capsule.toml is missing")
        stream = archive.extractfile(manifest_member)
        if stream is None:
            raise VerificationError(f"{path}: Capsule.toml is not readable")
        try:
            embedded_bytes = stream.read()
            embedded = tomllib.loads(embedded_bytes.decode("utf-8"))
        except (UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
            raise VerificationError(f"{path}: embedded Capsule.toml is invalid: {error}") from error
        if embedded_bytes != source_manifest_bytes:
            raise VerificationError(
                f"{path}: embedded Capsule.toml bytes differ from exact tagged source bytes"
            )
        if embedded != source_manifest:
            raise VerificationError(f"{path}: embedded Capsule.toml differs from exact source manifest")

        components = embedded.get("component")
        if not isinstance(components, list) or not components:
            raise VerificationError(f"{path}: at least one [[component]] is required")
        component_names: list[str] = []
        for component in components:
            if not isinstance(component, dict):
                raise VerificationError(f"{path}: component entry is not a table")
            filename = component.get("file", component.get("entrypoint"))
            if (
                not isinstance(filename, str)
                or not filename.endswith(".wasm")
                or PurePosixPath(filename).name != filename
            ):
                raise VerificationError(f"{path}: unsafe component filename {filename!r}")
            if filename in component_names:
                raise VerificationError(f"{path}: duplicate component filename {filename!r}")
            component_names.append(filename)
            member = by_name.get(filename)
            if member is None or not member.isfile():
                raise VerificationError(f"{path}: component {filename!r} is missing")

        expected_members = {"Capsule.toml", *component_names}
        if set(by_name) != expected_members:
            raise VerificationError(
                f"{path}: archive member set differs; missing={sorted(expected_members - set(by_name))}, "
                f"unexpected={sorted(set(by_name) - expected_members)}"
            )
        return {
            "directory": item.directory,
            "package": item.package,
            "version": item.version,
            "coordinate": item.coordinate,
            "asset": item.asset,
            "components": component_names,
            "sha256": _sha256(path),
            "size": path.stat().st_size,
        }


def verify_source_tree(source_root: Path) -> dict[str, str]:
    if not source_root.is_dir() or source_root.is_symlink():
        raise VerificationError(f"source root is not a directory: {source_root}")
    try:
        branch = subprocess.run(
            _git_command(
                "-C", str(source_root), "symbolic-ref", "--quiet", "--short", "HEAD"
            ),
            check=False,
            capture_output=True,
            text=True,
        )
        status = subprocess.run(
            _git_command("-C", str(source_root), "status", "--porcelain"),
            check=False,
            capture_output=True,
            text=True,
        )
        head = subprocess.run(
            _git_command("-C", str(source_root), "rev-parse", "HEAD^{commit}"),
            check=False,
            capture_output=True,
            text=True,
        )
        tree = subprocess.run(
            _git_command(
                "-C", str(source_root), "rev-parse", f"{policy.SOURCE_COMMIT}^{{tree}}"
            ),
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as error:
        raise VerificationError(f"cannot invoke git for source verification: {error}") from error
    if branch.returncode == 0 or branch.stdout.strip():
        raise VerificationError(
            "source checkout must be detached; a branch name cannot prove the immutable release tag"
        )
    if status.returncode != 0:
        raise VerificationError(f"cannot inspect source checkout status: {status.stderr.strip()}")
    if status.stdout.strip():
        raise VerificationError("source checkout is dirty; immutable release verification requires a clean tree")
    if head.returncode != 0:
        raise VerificationError(f"source root is not a git checkout: {source_root}")
    actual_head = head.stdout.strip()
    if actual_head != policy.SOURCE_COMMIT:
        raise VerificationError(
            f"source checkout HEAD {actual_head!r} is not pinned {policy.SOURCE_COMMIT}"
        )
    if tree.returncode != 0 or tree.stdout.strip() != policy.SOURCE_TREE:
        raise VerificationError(
            f"source tree does not resolve to pinned {policy.SOURCE_TREE}; "
            f"got {tree.stdout.strip()!r}"
        )
    return {"commit": actual_head, "tree": tree.stdout.strip(), "tag": policy.SOURCE_TAG}


def verify_current_main_drift(current_main_root: Path) -> dict[str, Any]:
    """Check current main independently from the detached release proof.

    This check is deliberately a separate input and result.  Current main is
    not evidence for the tagged release; it only proves that the three newer
    capsules remain outside the 19-package seed until a signed release exists.
    """
    if not current_main_root.is_dir() or current_main_root.is_symlink():
        raise VerificationError(f"current-main root is not a directory: {current_main_root}")
    allowlist_path = current_main_root / "release" / "community-capsules.txt"
    try:
        directories = [
            line.strip()
            for line in allowlist_path.read_text(encoding="utf-8").splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        ]
    except OSError as error:
        raise VerificationError(f"cannot read current-main capsule allowlist: {error}") from error
    expected = [item.directory for item in policy.SEED]
    extras = sorted(set(directories) - set(expected))
    required = sorted(policy.BLOCKED_DIRECTORIES)
    if not set(required).issubset(directories):
        raise VerificationError(
            "current-main drift check is missing blocked additions: "
            f"expected={required}, actual={directories}"
        )
    if directories == expected:
        raise VerificationError("current-main drift check unexpectedly matches the 19-package release seed")
    return {
        "repository": policy.SOURCE_REPOSITORY_SLUG,
        "allowlist": directories,
        "seed_directories": expected,
        "blocked_additions": extras,
        "separate_from_release_proof": True,
    }


def _verify_sigstore(
    artifacts: Path,
    payloads: list[str],
    *,
    cosign: str,
    skip: bool,
) -> dict[str, Any]:
    bundles = []
    failures: list[str] = []
    for name in payloads:
        payload = artifacts / name
        bundle = artifacts / f"{name}.sigstore.json"
        if not bundle.is_file() or bundle.is_symlink():
            failures.append(f"missing Sigstore bundle {bundle.name}")
            continue
        try:
            value = json.loads(bundle.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            failures.append(f"invalid Sigstore bundle {bundle.name}: {error}")
            continue
        if not isinstance(value, dict) or not value:
            failures.append(f"Sigstore bundle {bundle.name} is empty")
            continue
        if skip:
            bundles.append({"asset": name, "bundle": bundle.name, "verified": False, "skipped": True})
            continue
        try:
            completed = subprocess.run(
                [
                    cosign,
                    "verify-blob",
                    "--bundle",
                    str(bundle),
                    "--certificate-oidc-issuer",
                    policy.SIGSTORE_ISSUER,
                    "--certificate-identity",
                    policy.SOURCE_WORKFLOW_IDENTITY,
                    str(payload),
                ],
                check=False,
                capture_output=True,
                text=True,
            )
        except OSError as error:
            failures.append(f"cannot invoke {cosign}: {error}")
            continue
        if completed.returncode != 0:
            detail = completed.stderr.strip() or completed.stdout.strip() or "verification failed"
            failures.append(f"{name}: {detail}")
            continue
        bundles.append({"asset": name, "bundle": bundle.name, "verified": True, "skipped": False})
    if failures and not skip:
        raise VerificationError("Sigstore verification failed: " + "; ".join(failures))
    return {
        "issuer": policy.SIGSTORE_ISSUER,
        "identity": policy.SOURCE_WORKFLOW_IDENTITY,
        "verified": not skip and not failures and len(bundles) == len(payloads),
        "skipped": skip,
        "bundles": bundles,
        "failures": failures,
    }


def verify_release(
    artifacts: Path,
    source_root: Path,
    *,
    b3sum: str = "b3sum",
    cosign: str = "cosign",
    skip_sigstore: bool = False,
    current_main_root: Path | None = None,
) -> dict[str, Any]:
    """Verify the exact 2026.1.3 release and return a deterministic report."""
    if not artifacts.is_dir() or artifacts.is_symlink():
        raise VerificationError(f"release artifact directory is not a directory: {artifacts}")
    for entry in artifacts.iterdir():
        if entry.is_symlink() or not entry.is_file():
            raise VerificationError(f"release artifacts contain non-regular entry: {entry.name}")

    source_identity = verify_source_tree(source_root)
    drift = verify_current_main_drift(current_main_root) if current_main_root is not None else None
    metadata_name = f"unicity-aos-{policy.PRODUCT_VERSION}-release.toml"
    metadata_path = artifacts / metadata_name
    release_metadata = _release_metadata_module()
    try:
        metadata = release_metadata.validate_release(
            release_metadata.load(metadata_path), require_ready=True
        )
    except (KeyError, OSError, ValueError) as error:
        raise VerificationError(f"invalid release metadata: {error}") from error
    if metadata["version"] != policy.PRODUCT_VERSION or metadata["tag"] != policy.SOURCE_TAG:
        raise VerificationError("release metadata version/tag is not the pinned 2026.1.3 release")
    if metadata["source-commit"] != policy.SOURCE_COMMIT:
        raise VerificationError("release metadata source commit is not the pinned tag commit")
    if metadata["release-workflow-identity"] != policy.SOURCE_WORKFLOW_IDENTITY:
        raise VerificationError("release metadata workflow identity is not the pinned tag identity")

    target_assets = sorted(item["asset"] for item in metadata["targets"].values())
    capsule_assets = [item.asset for item in policy.SEED]
    payloads = sorted(
        set(capsule_assets)
        | set(target_assets)
        | {"BLAKE3SUMS.txt", "SHA256SUMS.txt", "runtime-compatibility.toml", metadata_name}
    )
    expected = set(payloads) | {f"{name}.sigstore.json" for name in payloads}
    actual = {path.name for path in artifacts.iterdir()}
    if actual != expected:
        raise VerificationError(
            "release asset set differs from the 2026.1.3 seed; "
            f"missing={sorted(expected - actual)}, unexpected={sorted(actual - expected)}"
        )

    sha256 = _checksum_manifest(artifacts / "SHA256SUMS.txt")
    blake3 = _checksum_manifest(artifacts / "BLAKE3SUMS.txt")
    checksummed = set(capsule_assets) | set(target_assets)
    if set(sha256) != checksummed:
        raise VerificationError("SHA256SUMS.txt does not cover exactly the 19 capsules and targets")
    if set(blake3) != checksummed:
        raise VerificationError("BLAKE3SUMS.txt does not cover exactly the 19 capsules and targets")

    source_compatibility = source_root / "release" / "runtime-compatibility.toml"
    try:
        if (artifacts / "runtime-compatibility.toml").read_bytes() != source_compatibility.read_bytes():
            raise VerificationError("published runtime compatibility differs from the exact tag source")
    except OSError as error:
        raise VerificationError(f"runtime compatibility evidence is unreadable: {error}") from error

    capsules: list[dict[str, Any]] = []
    for item in policy.SEED:
        manifest, manifest_bytes = _load_source_manifest(source_root, item)
        capsule = _verify_capsule(artifacts / item.asset, item, manifest, manifest_bytes)
        capsule["runtime"] = manifest.get("package", {}).get("astrid-version", "*")
        expected_sha = sha256[item.asset]
        if capsule["sha256"] != expected_sha:
            raise VerificationError(f"SHA-256 mismatch for {item.asset}")
        capsule["blake3"] = _blake3(artifacts / item.asset, b3sum)
        if capsule["blake3"] != blake3[item.asset]:
            raise VerificationError(f"BLAKE3 mismatch for {item.asset}")
        capsules.append(capsule)

    targets: list[dict[str, Any]] = []
    for item in metadata["targets"].values():
        asset = item["asset"]
        path = artifacts / asset
        actual_sha = _sha256(path)
        actual_b3 = _blake3(path, b3sum)
        if actual_sha != sha256[asset] or actual_b3 != blake3[asset]:
            raise VerificationError(f"target digest mismatch for {asset}")
        if item["sha256"] != actual_sha or item["blake3"] != actual_b3:
            raise VerificationError(f"release metadata digest mismatch for {asset}")
        if item["size"] != path.stat().st_size:
            raise VerificationError(f"release metadata size mismatch for {asset}")
        targets.append({"asset": asset, "sha256": actual_sha, "blake3": actual_b3, "size": path.stat().st_size})

    sigstore = _verify_sigstore(
        artifacts,
        payloads,
        cosign=cosign,
        skip=skip_sigstore,
    )
    return {
        "schema": "aos-station-adapter-verification-v1",
        "product": "unicity-aos-ce",
        "version": policy.PRODUCT_VERSION,
        "source": {
            **source_identity,
            "repository": policy.SOURCE_REPOSITORY,
            "repository_slug": policy.SOURCE_REPOSITORY_SLUG,
            "owner_id": policy.SOURCE_OWNER_ID,
            "repository_id": policy.SOURCE_REPOSITORY_ID,
            "workflow_identity": policy.SOURCE_WORKFLOW_IDENTITY,
        },
        "assets": {"payloads": payloads, "capsules": capsules, "targets": sorted(targets, key=lambda value: value["asset"])},
        "sigstore": sigstore,
        "seed_count": len(capsules),
        "blocked": sorted(policy.BLOCKED_DIRECTORIES.items()),
        "current_main_drift": drift,
    }
