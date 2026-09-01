"""Shared parsing for the private Linux Realm capsule lineage tools."""

from __future__ import annotations

import hashlib
import subprocess
import tarfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST_PATH = ROOT / "Capsule.toml"
LOCK_PATH = ROOT / "assets/linux-vcpu.lock"
LINUX_SOURCES_LOCK_PATH = ROOT / "linux/SOURCES.lock"
SOURCE_SHA_SECTION_NAME = b"aos_source_sha"
EXPECTED_BUILDER_OCI = "ubuntu@sha256:c4a8d5503dfb2a3eb8ab5f807da5bc69a85730fb49b5cfca2330194ebcc41c7b"


class LineageError(RuntimeError):
    pass


def _read_unsigned_leb128(data: bytes, offset: int) -> tuple[int, int]:
    value = 0
    shift = 0
    while offset < len(data):
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7f) << shift
        if not byte & 0x80:
            return value, offset
        shift += 7
        if shift >= 64:
            break
    raise LineageError("truncated WebAssembly LEB128 value")


def read_source_sha_section(wasm: bytes) -> str:
    if len(wasm) < 8 or wasm[:4] != b"\0asm":
        raise LineageError("controller artifact is not WebAssembly")
    if wasm[8] != 0:
        raise LineageError("controller source-SHA custom section is missing")
    section_size, offset = _read_unsigned_leb128(wasm, 9)
    section_end = offset + section_size
    if section_end > len(wasm):
        raise LineageError("controller source-SHA custom section is truncated")
    payload = wasm[offset:section_end]
    name_size, payload_offset = _read_unsigned_leb128(payload, 0)
    name_end = payload_offset + name_size
    if name_end > len(payload):
        raise LineageError("controller source-SHA custom section name is truncated")
    if payload[payload_offset:name_end] != SOURCE_SHA_SECTION_NAME:
        raise LineageError("first controller custom section is not aos_source_sha")
    source = payload[name_end:].decode("ascii")
    if len(source) != 40 or any(character not in "0123456789abcdef" for character in source):
        raise LineageError("controller source-SHA custom section is malformed")
    return source


def source_sha_custom_section(source: str) -> bytes:
    if len(source) != 40 or any(character not in "0123456789abcdef" for character in source):
        raise LineageError("source SHA is not a lowercase commit SHA")
    name_length = len(SOURCE_SHA_SECTION_NAME)
    payload = bytes([name_length]) + SOURCE_SHA_SECTION_NAME + source.encode("ascii")
    return b"\x00" + bytes([len(payload)]) + payload


def locked_builder_metadata() -> dict:
    def value(name: str) -> str:
        for line in LINUX_SOURCES_LOCK_PATH.read_text(encoding="utf-8").splitlines():
            if line.startswith(f"{name}="):
                return line.removeprefix(f"{name}=")
        raise LineageError(f"{LINUX_SOURCES_LOCK_PATH} is missing {name}")

    builder_oci = value("builder_oci")
    if builder_oci != EXPECTED_BUILDER_OCI:
        raise LineageError(
            f"builder OCI mismatch: expected {EXPECTED_BUILDER_OCI}, got {builder_oci}"
        )
    return {
        "oci": builder_oci,
        "toolchain_pins": {
            "buildroot": value("buildroot_version"),
            "make": value("make"),
            "gcc": value("gcc"),
            "llvm": value("llvm"),
            "clang": value("clang"),
            "lld": value("lld"),
            "guest_rust": value("realm_rust"),
            "guest_rust_host": value("realm_rust_host"),
            "astrid_build": value("realm_astrid_build"),
        },
    }


def blake3_file(path: Path) -> str:
    try:
        result = subprocess.run(
            ["b3sum", str(path)],
            check=True,
            text=True,
            capture_output=True,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        raise LineageError(f"unable to BLAKE3-hash {path}: {error}") from error
    return result.stdout.split(" ", 1)[0]


def blake3_bytes(data: bytes) -> str:
    process = subprocess.Popen(
        ["b3sum"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    stdout, stderr = process.communicate(data)
    if process.returncode != 0:
        raise LineageError(f"unable to BLAKE3-hash bytes: {stderr}")
    return stdout.decode("ascii").split(" ", 1)[0]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def manifest() -> dict:
    try:
        return tomllib.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise LineageError(f"invalid {MANIFEST_PATH}: {error}") from error


def archive_members(path: Path) -> dict[str, tuple[tarfile.TarInfo, bytes]]:
    if not path.is_file():
        raise LineageError(f"missing builder archive: {path}")

    members: dict[str, tuple[tarfile.TarInfo, bytes]] = {}
    try:
        with tarfile.open(path, mode="r:gz") as archive:
            for info in archive:
                if info.isdir():
                    continue
                if not info.isreg():
                    raise LineageError(f"builder emitted non-regular member {info.name}")
                if info.name in members:
                    raise LineageError(f"builder emitted duplicate member {info.name}")
                members[info.name] = (info, archive.extractfile(info).read())
    except (OSError, tarfile.TarError) as error:
        raise LineageError(f"invalid builder archive {path}: {error}") from error
    return members


def declared_members(spec: dict) -> list[str]:
    paths: list[str] = []
    for component in spec.get("component", []):
        path = component.get("file")
        if not isinstance(path, str) or not path:
            raise LineageError("component is missing file")
        paths.append(path)
        for asset in component.get("asset", []):
            asset_path = asset.get("file")
            if not isinstance(asset_path, str) or not asset_path:
                raise LineageError("component asset is missing file")
            paths.append(asset_path)
    return paths


def expected_builder_members(spec: dict) -> set[str]:
    expected = {"Capsule.toml", *declared_members(spec), LOCK_PATH.relative_to(ROOT).as_posix()}
    return expected


def validate_declared_hashes(spec: dict, content_by_path: dict[str, bytes]) -> None:
    for component in spec.get("component", []):
        path = component["file"]
        if component.get("type") == "executable" and component.get("hash") is None:
            continue
        _validate_hash(path, component.get("hash"), content_by_path[path])
        for asset in component.get("asset", []):
            path = asset["file"]
            _validate_hash(path, asset.get("hash"), content_by_path[path])


def _validate_hash(path: str, recorded: object, content: bytes) -> None:
    if not isinstance(recorded, str) or not recorded.startswith("blake3:"):
        raise LineageError(f"{path} has no blake3 manifest hash")
    actual = blake3_bytes(content)
    expected = recorded.removeprefix("blake3:")
    if actual != expected:
        raise LineageError(f"{path} hash mismatch: expected {expected}, got {actual}")
