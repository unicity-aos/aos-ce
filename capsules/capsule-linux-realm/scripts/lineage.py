"""Shared parsing for the private Linux Realm capsule lineage tools."""

from __future__ import annotations

import hashlib
import json
import base64
import struct
import subprocess
import tarfile
import tomllib
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST_PATH = ROOT / "Capsule.toml"
LOCK_PATH = ROOT / "assets/linux-vcpu.lock"
LINUX_SOURCES_LOCK_PATH = ROOT / "linux/SOURCES.lock"
SOURCE_SHA_SECTION_NAME = b"aos_source_sha"
EXPECTED_BUILDER_OCI = "ubuntu@sha256:c4a8d5503dfb2a3eb8ab5f807da5bc69a85730fb49b5cfca2330194ebcc41c7b"
PROVENANCE_MEMBER = "Capsule.provenance.json"
PROVENANCE_SCHEMA_VERSION = 1
PROVENANCE_ALGORITHM = "ed25519-blake3-tree-v1"
PROVENANCE_CONTENT_DOMAIN = b"astrid:capsule-content:v1\0"
PROVENANCE_SIGNATURE_DOMAIN = b"astrid:capsule-provenance:v1\0"
_ED25519_SPKI_PREFIX = bytes.fromhex("302a300506032b6570032100")


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
        "host_tools_snapshot": {
            "id": value("host_tools_snapshot_id"),
            "uri": value("host_tools_snapshot_uri"),
            "suites": value("host_tools_suites"),
            "components": value("host_tools_components"),
            "archive_key_fingerprint": value("host_tools_archive_key_fingerprint"),
        },
        "toolchain_pins": {
            "buildroot": value("buildroot_version"),
            "make": value("make"),
            "gcc": value("gcc"),
            "gpp": value("gpp"),
            "cpp13": value("cpp13"),
            "gcc13": value("gcc13"),
            "gpp13": value("gpp13"),
            "gcc13_base": value("gcc13_base"),
            "gcc13_aarch64": value("gcc13_aarch64"),
            "gpp13_aarch64": value("gpp13_aarch64"),
            "cpp13_aarch64": value("cpp13_aarch64"),
            "libgcc13_dev": value("libgcc13_dev"),
            "libstdcpp13_dev": value("libstdcpp13_dev"),
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
                validate_archive_member_name(info.name)
                if info.isdir():
                    continue
                if not info.isreg():
                    raise LineageError(f"builder emitted non-regular member {info.name}")
                if info.pax_headers:
                    raise LineageError(f"builder member {info.name} has PAX metadata")
                if info.name in members:
                    raise LineageError(f"builder emitted duplicate member {info.name}")
                if info.name == PROVENANCE_MEMBER and info.size > 64 * 1024:
                    raise LineageError("builder provenance envelope exceeds 64 KiB")
                members[info.name] = (info, archive.extractfile(info).read())
    except (OSError, tarfile.TarError) as error:
        raise LineageError(f"invalid builder archive {path}: {error}") from error
    return members


def read_pinned_source_member(path: Path, root: Path) -> bytes:
    """Read a regular checkout member without following symlinks below the root."""
    try:
        relative = path.relative_to(root)
    except ValueError as error:
        raise LineageError(f"source member is outside the checkout: {path}") from error
    if not relative.parts or any(part in ("", ".", "..") for part in relative.parts):
        raise LineageError(f"invalid source member path: {path}")
    current = root
    for part in relative.parts:
        current = current / part
        if current.is_symlink():
            raise LineageError(f"source member path contains a symlink: {path}")
    if not path.is_file():
        raise LineageError(f"missing declared source member: {path}")
    return path.read_bytes()


def validate_archive_member_name(name: str) -> None:
    """Use the same fail-closed canonical path shape as Astrid archives."""
    if (
        not name
        or name.startswith("/")
        or "\\" in name
        or "\0" in name
        or any(part in ("", ".", "..") for part in name.split("/"))
        or (len(name) >= 2 and name[0].isalpha() and name[1] == ":")
    ):
        raise LineageError(f"non-canonical capsule member path: {name}")


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
    expected = {
        "Capsule.toml",
        *declared_members(spec),
        LOCK_PATH.relative_to(ROOT).as_posix(),
        PROVENANCE_MEMBER,
    }
    return expected


def _hex_bytes(value: object, length: int, label: str) -> bytes:
    if (
        not isinstance(value, str)
        or len(value) != length * 2
        or any(character not in "0123456789abcdef" for character in value)
    ):
        raise LineageError(f"provenance {label} must be {length * 2} lowercase hex characters")
    return bytes.fromhex(value)


def _base64_bytes(value: object, length: int, label: str) -> bytes:
    if not isinstance(value, str):
        raise LineageError(f"provenance {label} must be base64 text")
    try:
        decoded = base64.b64decode(value, validate=True)
    except (ValueError, base64.binascii.Error) as error:
        raise LineageError(f"provenance {label} is not canonical base64") from error
    if len(decoded) != length:
        raise LineageError(f"provenance {label} must decode to {length} bytes")
    return decoded


def content_tree_digest(members: dict[str, bytes]) -> str:
    """Mirror the Astrid canonical member-tree digest contract."""
    framed = bytearray(PROVENANCE_CONTENT_DOMAIN)
    for name in sorted(members):
        data = members[name]
        path = name.encode("utf-8")
        framed.extend(struct.pack("<Q", len(path)))
        framed.extend(path)
        framed.extend(struct.pack("<Q", len(data)))
        framed.extend(bytes.fromhex(blake3_bytes(data)))
    return blake3_bytes(framed)


def _verify_ed25519(public_key: bytes, message: bytes, signature: bytes) -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        public_path = root / "public.spki"
        message_path = root / "message.bin"
        signature_path = root / "signature.bin"
        public_path.write_bytes(_ED25519_SPKI_PREFIX + public_key)
        message_path.write_bytes(message)
        signature_path.write_bytes(signature)
        try:
            result = subprocess.run(
                [
                    "openssl",
                    "pkeyutl",
                    "-verify",
                    "-pubin",
                    "-keyform",
                    "DER",
                    "-inkey",
                    str(public_path),
                    "-rawin",
                    "-in",
 str(message_path),
                    "-sigfile",
 str(signature_path),
                ],
                check=False,
                text=True,
                capture_output=True,
            )
        except OSError as error:
            raise LineageError(f"unable to run OpenSSL Ed25519 verifier: {error}") from error
        if result.returncode != 0:
            raise LineageError(
                "provenance signature verification failed: "
                f"{result.stdout.strip()} {result.stderr.strip()}".strip()
            )


def verify_builder_provenance(
    members: dict[str, tuple[tarfile.TarInfo, bytes]],
    *,
    expected_signer: str,
    label: str = "builder",
) -> dict:
    """Verify the builder envelope against its own archive, then discard it."""
    if PROVENANCE_MEMBER not in members:
        raise LineageError(f"{label} archive is missing {PROVENANCE_MEMBER}")
    _, envelope_bytes = members[PROVENANCE_MEMBER]
    if len(envelope_bytes) > 64 * 1024:
        raise LineageError(f"{label} provenance envelope exceeds 64 KiB")
    try:
        envelope = json.loads(envelope_bytes)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise LineageError(f"invalid {label} provenance envelope: {error}") from error
    if not isinstance(envelope, dict):
        raise LineageError(f"{label} provenance envelope must be an object")
    if (
        envelope.get("schema_version") != PROVENANCE_SCHEMA_VERSION
        or envelope.get("algorithm") != PROVENANCE_ALGORITHM
    ):
        raise LineageError(f"unsupported {label} provenance schema or algorithm")
    content_digest = envelope.get("content_digest")
    if not isinstance(content_digest, str):
        raise LineageError(f"{label} provenance content digest must be a string")
    _hex_bytes(content_digest, 32, "content digest")
    signer = _base64_bytes(envelope.get("signer"), 32, "signer")
    if signer != _base64_bytes(expected_signer, 32, "expected signer"):
        raise LineageError(f"{label} signer differs from the trusted signer pin")
    signature = _base64_bytes(envelope.get("signature"), 64, "signature")
    builder_content = {
        name: data for name, (_, data) in members.items() if name != PROVENANCE_MEMBER
    }
    actual_digest = content_tree_digest(builder_content)
    if content_digest != actual_digest:
        raise LineageError(
            f"{label} provenance digest mismatch: expected {content_digest}, "
            f"got {actual_digest}"
        )
    message = PROVENANCE_SIGNATURE_DOMAIN + content_digest.encode("ascii")
    _verify_ed25519(signer, message, signature)
    return {
        "schema_version": envelope["schema_version"],
        "algorithm": envelope["algorithm"],
        "content_digest": content_digest,
        "signer": envelope["signer"],
        "signature": envelope["signature"],
    }


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
