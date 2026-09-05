#!/usr/bin/env python3
"""Validate the Community Edition capsule source and release artifacts."""

from __future__ import annotations

import argparse
import io
import json
import re
import stat
import sys
import tarfile
import unicodedata
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

try:
    import tomllib
except ModuleNotFoundError:  # Python 3.10 and older
    import tomli as tomllib


ROOT = Path(__file__).resolve().parent.parent
POLICY = ROOT / "release" / "community-capsules.txt"
DISTRO = ROOT / "distros" / "community" / "unicity-ce" / "Distro.toml"
PROVENANCE_FILE = "Capsule.provenance.json"
PROVENANCE_KEYS = frozenset(
    {"schema_version", "algorithm", "content_digest", "signer", "signature"}
)
DURABLE_DIRECTORY_MEMBERS = frozenset(
    {"wit", "wit/deps", "wit/deps/astrid-contracts"}
)
DURABLE_FILE_MEMBERS = frozenset(
    {
        "Capsule.toml",
        PROVENANCE_FILE,
        "wit/capsule.wit",
        "wit/deps/astrid-contracts/astrid-contracts.wit",
    }
)


class ContractError(RuntimeError):
    """The capsule release contract is inconsistent or unsafe."""


@dataclass(frozen=True)
class CapsuleSpec:
    directory: str
    package: str
    version: str
    components: tuple[str, ...]
    manifest: Path

    @property
    def asset(self) -> str:
        return f"{self.package}.capsule"


def load_toml(path: Path) -> dict:
    try:
        return tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ContractError(f"cannot parse {path}: {error}") from error


def release_directories() -> list[str]:
    try:
        lines = POLICY.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise ContractError(f"cannot read {POLICY}: {error}") from error

    directories = [line.strip() for line in lines if line.strip() and not line.lstrip().startswith("#")]
    if not directories:
        raise ContractError("community capsule allowlist is empty")
    if len(directories) != len(set(directories)):
        raise ContractError("community capsule allowlist contains duplicates")
    for directory in directories:
        if not re.fullmatch(r"capsule-[a-z0-9-]+", directory):
            raise ContractError(f"invalid capsule directory in allowlist: {directory}")
    return directories


def source_contract() -> list[CapsuleSpec]:
    workspace = load_toml(ROOT / "Cargo.toml").get("workspace", {})
    members = set(workspace.get("members", []))
    distro = load_toml(DISTRO)
    distro_entries = distro.get("capsule", [])
    if not isinstance(distro_entries, list):
        raise ContractError(f"{DISTRO}: [[capsule]] entries are missing")

    distro_by_name: dict[str, dict] = {}
    for entry in distro_entries:
        name = entry.get("name") if isinstance(entry, dict) else None
        if not isinstance(name, str) or not name:
            raise ContractError(f"{DISTRO}: capsule entry has no name")
        if name in distro_by_name:
            raise ContractError(f"{DISTRO}: duplicate capsule {name}")
        distro_by_name[name] = entry

    specs: list[CapsuleSpec] = []
    for directory in release_directories():
        member = f"capsules/{directory}"
        if member not in members:
            raise ContractError(f"release capsule is not a workspace member: {member}")

        root = ROOT / member
        cargo_path = root / "Cargo.toml"
        manifest_path = root / "Capsule.toml"
        cargo_package = load_toml(cargo_path).get("package", {})
        manifest = load_toml(manifest_path)
        manifest_package = manifest.get("package", {})

        cargo_name = cargo_package.get("name")
        cargo_version = cargo_package.get("version")
        manifest_name = manifest_package.get("name")
        manifest_version = manifest_package.get("version")
        if not all(isinstance(value, str) and value for value in (cargo_name, cargo_version, manifest_name, manifest_version)):
            raise ContractError(f"{member}: package name/version is incomplete")
        if cargo_name != manifest_name or cargo_version != manifest_version:
            raise ContractError(
                f"{member}: Cargo package {cargo_name} {cargo_version} does not match "
                f"Capsule package {manifest_name} {manifest_version}"
            )
        if not cargo_name.startswith("aos-"):
            raise ContractError(f"{member}: published package identity must use aos-*")
        expected_name = f"aos-{directory.removeprefix('capsule-')}"
        if cargo_name != expected_name:
            raise ContractError(
                f"{member}: published package identity must be {expected_name}, got {cargo_name}"
            )

        component_entries = manifest.get("component")
        if not isinstance(component_entries, list) or not component_entries:
            raise ContractError(f"{manifest_path}: at least one [[component]] is required")
        components: list[str] = []
        for component in component_entries:
            filename = component.get("file") if isinstance(component, dict) else None
            if (
                not isinstance(filename, str)
                or not filename.endswith(".wasm")
                or PurePosixPath(filename).name != filename
            ):
                raise ContractError(f"{manifest_path}: unsafe component filename {filename!r}")
            components.append(filename)
        if len(components) != len(set(components)):
            raise ContractError(f"{manifest_path}: duplicate component filename")

        distro_entry = distro_by_name.get(cargo_name)
        if distro_entry is None:
            raise ContractError(f"{member}: {cargo_name} is not selected by Unicity CE")
        if distro_entry.get("version") != cargo_version:
            raise ContractError(
                f"{DISTRO}: {cargo_name} version {distro_entry.get('version')!r} "
                f"does not match source {cargo_version}"
            )
        expected_source = f"capsules/{cargo_name}.capsule"
        if distro_entry.get("source") != expected_source:
            raise ContractError(
                f"{DISTRO}: {cargo_name} source must be the bundled asset {expected_source!r}"
            )

        specs.append(
            CapsuleSpec(
                directory=directory,
                package=cargo_name,
                version=cargo_version,
                components=tuple(components),
                manifest=manifest_path,
            )
        )

    expected_names = {spec.package for spec in specs}
    if set(distro_by_name) != expected_names:
        missing = sorted(set(distro_by_name) - expected_names)
        extra = sorted(expected_names - set(distro_by_name))
        raise ContractError(
            f"community capsule allowlist and distro differ; unlisted={missing}, not-in-distro={extra}"
        )
    return specs


def _canonical_member_name(member: tarfile.TarInfo, asset: Path) -> str:
    """Return the extraction path, rejecting aliases and unsafe names."""
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
        raise ContractError(f"{asset}: unsafe or non-canonical archive path {name!r}")
    return raw


def _read_embedded_manifest(archive: tarfile.TarFile, member: tarfile.TarInfo, asset: Path) -> dict:
    stream = archive.extractfile(member)
    if stream is None:
        raise ContractError(f"{asset}: Capsule.toml is not a regular file")
    try:
        return tomllib.loads(stream.read().decode("utf-8"))
    except (UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
        raise ContractError(f"{asset}: embedded Capsule.toml is invalid: {error}") from error


def _reject_duplicate_json_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate key {key!r}")
        result[key] = value
    return result


def _reject_json_constant(value: str) -> None:
    raise ValueError(f"invalid JSON constant {value!r}")


def _read_provenance(archive: tarfile.TarFile, member: tarfile.TarInfo, asset: Path) -> None:
    if member.size > 64 * 1024:
        raise ContractError(f"{asset}: {PROVENANCE_FILE} exceeds 64 KiB")
    stream = archive.extractfile(member)
    if stream is None:
        raise ContractError(f"{asset}: {PROVENANCE_FILE} is not a regular file")
    try:
        envelope = json.loads(
            stream.read().decode("utf-8"),
            object_pairs_hook=_reject_duplicate_json_keys,
            parse_constant=_reject_json_constant,
        )
    except (UnicodeDecodeError, ValueError) as error:
        raise ContractError(f"{asset}: {PROVENANCE_FILE} is invalid JSON: {error}") from error
    if not isinstance(envelope, dict):
        raise ContractError(f"{asset}: {PROVENANCE_FILE} must contain a JSON object")
    if set(envelope) != PROVENANCE_KEYS:
        missing = sorted(PROVENANCE_KEYS - set(envelope))
        unexpected = sorted(set(envelope) - PROVENANCE_KEYS)
        raise ContractError(
            f"{asset}: {PROVENANCE_FILE} keys differ; missing={missing}, unexpected={unexpected}"
        )
    if type(envelope["schema_version"]) is not int or envelope["schema_version"] != 1:
        raise ContractError(f"{asset}: {PROVENANCE_FILE} schema_version must be integer 1")
    if envelope["algorithm"] != "ed25519-blake3-tree-v1":
        raise ContractError(
            f"{asset}: {PROVENANCE_FILE} algorithm must be 'ed25519-blake3-tree-v1'"
        )
    content_digest = envelope["content_digest"]
    if not isinstance(content_digest, str) or re.fullmatch(r"[0-9a-f]{64}", content_digest) is None:
        raise ContractError(
            f"{asset}: {PROVENANCE_FILE} content_digest must be 64 lowercase hexadecimal characters"
        )
    for key in ("algorithm", "signer", "signature"):
        if not isinstance(envelope[key], str) or not envelope[key]:
            raise ContractError(f"{asset}: {PROVENANCE_FILE} {key} must be a non-empty string")


def validate_archive(asset: Path, spec: CapsuleSpec) -> None:
    try:
        archive = tarfile.open(asset, mode="r:gz")
    except (OSError, tarfile.TarError) as error:
        raise ContractError(f"{asset}: invalid capsule archive: {error}") from error

    with archive:
        members = archive.getmembers()
        names: dict[str, tarfile.TarInfo] = {}
        portability_names: dict[str, str] = {}
        for member in members:
            if not (member.isfile() or member.isdir()):
                raise ContractError(f"{asset}: links and special files are forbidden ({member.name})")
            canonical_name = _canonical_member_name(member, asset)
            if canonical_name in names:
                raise ContractError(f"{asset}: duplicate archive path {canonical_name!r}")
            portability_name = unicodedata.normalize("NFC", canonical_name).casefold()
            if portability_name in portability_names:
                raise ContractError(
                    f"{asset}: archive paths collide on portable filesystems: "
                    f"{portability_names[portability_name]!r} and {canonical_name!r}"
                )
            names[canonical_name] = member
            portability_names[portability_name] = canonical_name

        expected_members = {
            *DURABLE_DIRECTORY_MEMBERS,
            *DURABLE_FILE_MEMBERS,
            *spec.components,
        }
        if set(names) != expected_members:
            raise ContractError(
                f"{asset}: archive member set differs; "
                f"missing={sorted(expected_members - set(names))}, "
                f"unexpected={sorted(set(names) - expected_members)}"
            )

        for name in DURABLE_DIRECTORY_MEMBERS:
            if not names[name].isdir():
                raise ContractError(f"{asset}: archive member {name!r} must be a directory")
        for name in (*DURABLE_FILE_MEMBERS, *spec.components):
            if not names[name].isfile():
                raise ContractError(f"{asset}: archive member {name!r} must be a regular file")

        manifest_member = names.get("Capsule.toml")
        if manifest_member is None or not manifest_member.isfile():
            raise ContractError(f"{asset}: Capsule.toml is missing")
        embedded = _read_embedded_manifest(archive, manifest_member, asset)
        source_manifest = load_toml(spec.manifest)
        if embedded != source_manifest:
            raise ContractError(f"{asset}: embedded Capsule.toml does not match source manifest")
        package = embedded.get("package", {})
        if package.get("name") != spec.package or package.get("version") != spec.version:
            raise ContractError(
                f"{asset}: embedded package identity does not match {spec.package} {spec.version}"
            )

        embedded_components = embedded.get("component")
        if not isinstance(embedded_components, list):
            raise ContractError(f"{asset}: embedded [[component]] entries are missing")
        component_files = tuple(
            component.get("file") for component in embedded_components if isinstance(component, dict)
        )
        if component_files != spec.components:
            raise ContractError(f"{asset}: embedded component list does not match source manifest")
        for component in spec.components:
            member = names.get(component)
            if member is None or not member.isfile():
                raise ContractError(f"{asset}: component {component} is missing")

        _read_provenance(archive, names[PROVENANCE_FILE], asset)
        for name in ("wit/capsule.wit", "wit/deps/astrid-contracts/astrid-contracts.wit"):
            if names[name].size == 0:
                raise ContractError(f"{asset}: archive member {name!r} must not be empty")


def validate_artifacts(directory: Path, specs: list[CapsuleSpec]) -> None:
    if directory.is_symlink() or not directory.is_dir():
        raise ContractError(f"capsule artifact directory does not exist: {directory}")
    expected = {spec.asset for spec in specs}
    entries = list(directory.iterdir())
    invalid = sorted(
        path.name
        for path in entries
        if path.is_symlink() or not stat.S_ISREG(path.lstat().st_mode)
    )
    if invalid:
        raise ContractError(f"capsule artifact directory contains non-regular entries: {invalid}")
    actual = {path.name for path in entries}
    if actual != expected:
        raise ContractError(
            f"capsule artifact set differs; missing={sorted(expected - actual)}, "
            f"unexpected={sorted(actual - expected)}"
        )
    for spec in specs:
        validate_archive(directory / spec.asset, spec)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifacts", type=Path, help="validate the exact built .capsule set")
    parser.add_argument("--print-build-plan", action="store_true", help="print directory and package TSV")
    parser.add_argument("--print-assets", action="store_true", help="print canonical capsule asset names")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    try:
        specs = source_contract()
        args = parse_args(argv)
        if args.artifacts is not None:
            validate_artifacts(args.artifacts, specs)
        if args.print_build_plan:
            for spec in specs:
                print(f"{spec.directory}\t{spec.package}")
        if args.print_assets:
            for spec in specs:
                print(spec.asset)
    except ContractError as error:
        print(f"capsule release contract error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
