"""Canonical packaging keeps builder provenance honest and never silent."""

from __future__ import annotations

import gzip
import base64
import io
import json
import os
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

import canonical_package
import lineage
import validate_capsule


SOURCE_SHA = "0123456789abcdef0123456789abcdef01234567"


def _run(arguments: list[str], *, data: bytes | None = None) -> bytes:
    result = subprocess.run(
        arguments,
        input=data,
        check=True,
        capture_output=True,
    )
    return result.stdout


class TestKey:
    def __init__(self, root: Path) -> None:
        self.private = root / "signing-key.pem"
        _run(["openssl", "genpkey", "-algorithm", "ed25519", "-out", str(self.private)])
        public_der = _run(
            ["openssl", "pkey", "-in", str(self.private), "-pubout", "-outform", "DER"]
        )
        self.public_b64 = base64.b64encode(public_der[-32:]).decode("ascii")

    def sign(self, message: bytes) -> str:
        message_path = self.private.with_suffix(".message")
        message_path.write_bytes(message)
        signature = _run(
            [
                "openssl",
                "pkeyutl",
                "-sign",
                "-inkey",
                str(self.private),
                "-rawin",
                "-in",
                str(message_path),
            ]
        )
        return base64.b64encode(signature).decode("ascii")


def signed_envelope(members: dict[str, bytes], key: TestKey) -> bytes:
    content = {
        name: data for name, data in members.items() if name != lineage.PROVENANCE_MEMBER
    }
    digest = lineage.content_tree_digest(content)
    envelope = {
        "schema_version": 1,
        "algorithm": "ed25519-blake3-tree-v1",
        "content_digest": digest,
        "signer": key.public_b64,
        "signature": key.sign(lineage.PROVENANCE_SIGNATURE_DOMAIN + digest.encode("ascii")),
    }
    return json.dumps(envelope, indent=2).encode("utf-8")


def builder_archive(path: Path, members: dict[str, bytes]) -> None:
    with path.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", mtime=0, fileobj=raw) as compressed:
            with tarfile.open(fileobj=compressed, mode="w") as archive:
                for name in sorted(members):
                    data = members[name]
                    info = tarfile.TarInfo(name)
                    info.size = len(data)
                    info.mode = 0o644
                    archive.addfile(info, io.BytesIO(data))


def canonical_archive(path: Path, members: dict[str, bytes]) -> None:
    with path.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", mtime=0, fileobj=raw) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.GNU_FORMAT) as archive:
                for name in sorted(members):
                    data = members[name]
                    info = tarfile.TarInfo(name)
                    info.size = len(data)
                    info.mode = 0o644
                    info.uid = 0
                    info.gid = 0
                    info.mtime = 0
                    info.uname = "root"
                    info.gname = "root"
                    archive.addfile(info, io.BytesIO(data))


class CanonicalPackagingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "assets").mkdir()
        (self.root / "linux").mkdir()
        self.key = TestKey(self.root)

        self.controller = b"\0asm\x01\0\0\0controller"
        self.worker = b"pinned worker"
        self.kernel = b"pinned kernel"
        (self.root / "assets/aos_linux_realm.wasm").write_bytes(self.controller)
        (self.root / "assets/worker.wasm").write_bytes(self.worker)
        (self.root / "assets/kernel.img").write_bytes(self.kernel)
        manifest = f"""[package]
name = "realm"
version = "1.0.0"

[[component]]
id = "controller"
file = "aos_linux_realm.wasm"
type = "executable"

[[component]]
id = "worker"
file = "assets/worker.wasm"
type = "compute-worker"
hash = "blake3:{lineage.blake3_bytes(self.worker)}"

[[component.asset]]
id = "kernel"
file = "assets/kernel.img"
hash = "blake3:{lineage.blake3_bytes(self.kernel)}"
"""
        manifest_path = self.root / "Capsule.toml"
        manifest_path.write_text(manifest)
        lock_path = self.root / "assets/linux-vcpu.lock"
        lock_path.write_bytes(b"worker-lock\n")
        sources_lock = self.root / "linux/SOURCES.lock"
        values = [
            "builder_oci=" + lineage.EXPECTED_BUILDER_OCI,
            "host_tools_snapshot_id=snapshot",
            "host_tools_snapshot_uri=uri",
            "host_tools_suites=suite",
            "host_tools_components=component",
            "host_tools_archive_key_fingerprint=fingerprint",
            "buildroot_version=2026.05.1",
            "make=make",
            "gcc=gcc",
            "gpp=g++",
            "cpp13=cpp13",
            "gcc13=gcc13",
            "gpp13=g++13",
            "gcc13_base=base",
            "gcc13_aarch64=arm",
            "gpp13_aarch64=arm++",
            "cpp13_aarch64=arm-cpp",
            "libgcc13_dev=libgcc",
            "libstdcpp13_dev=libstdc++",
            "llvm=llvm",
            "clang=clang",
            "lld=lld",
            "realm_rust=rust",
            "realm_rust_host=host",
            "realm_astrid_build=build",
        ]
        sources_lock.write_text("\n".join(values) + "\n")

        self.builder_members: dict[str, bytes] = {
            "Capsule.toml": manifest_path.read_bytes(),
            "aos_linux_realm.wasm": self.controller,
            "assets/worker.wasm": self.worker,
            "assets/kernel.img": self.kernel,
            "assets/linux-vcpu.lock": lock_path.read_bytes(),
            "wit/capsule.wit": b"package witness contract\n",
        }
        self.builder_path = self.root / "builder.capsule"
        self.resign_builder()

    def resign_builder(self) -> None:
        unsigned = {
            name: data for name, data in self.builder_members.items()
            if name != lineage.PROVENANCE_MEMBER
        }
        self.builder_members = {
            **unsigned,
            lineage.PROVENANCE_MEMBER: signed_envelope(unsigned, self.key),
        }
        builder_archive(self.builder_path, self.builder_members)

    def package(self) -> tuple[Path, dict]:
        output = self.root / "dist/realm.capsule"
        patches = [
            patch.object(canonical_package, "ROOT", self.root),
            patch.object(canonical_package, "MANIFEST_PATH", self.root / "Capsule.toml"),
            patch.object(canonical_package, "LOCK_PATH", self.root / "assets/linux-vcpu.lock"),
            patch.object(lineage, "ROOT", self.root),
            patch.object(lineage, "MANIFEST_PATH", self.root / "Capsule.toml"),
            patch.object(lineage, "LOCK_PATH", self.root / "assets/linux-vcpu.lock"),
            patch.object(lineage, "LINUX_SOURCES_LOCK_PATH", self.root / "linux/SOURCES.lock"),
            patch.object(validate_capsule, "ROOT", self.root),
            patch.object(validate_capsule, "MANIFEST_PATH", self.root / "Capsule.toml"),
            patch.object(validate_capsule, "LOCK_PATH", self.root / "assets/linux-vcpu.lock"),
            patch.dict(os.environ, {"AOS_SOURCE_SHA": SOURCE_SHA}),
        ]
        for active in patches:
            active.start()
            self.addCleanup(active.stop)
        record = canonical_package.package(
            self.builder_path,
            output,
            allow_unsigned=True,
            expected_builder_signer=self.key.public_b64,
        )
        return output, record

    def test_verifies_builder_envelope_and_emits_explicit_unsigned_canonical(self) -> None:
        output, record = self.package()
        members = validate_capsule.read_members(output)
        self.assertNotIn(lineage.PROVENANCE_MEMBER, members)
        self.assertEqual(members["Capsule.toml"], (self.root / "Capsule.toml").read_bytes())
        self.assertEqual(
            record["builder_provenance"]["content_digest"],
            lineage.content_tree_digest(
                {
                    name: data for name, data in self.builder_members.items()
                    if name != lineage.PROVENANCE_MEMBER
                }
            ),
        )
        self.assertEqual(record["canonical_provenance"]["state"], "unsigned")
        self.assertEqual(
            record["unsigned_content_digest"], lineage.content_tree_digest(members)
        )

        lineage_path = output.with_suffix(output.suffix + ".lineage.json")
        lineage_path.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")
        validate_capsule.validate(
            output, lineage_path, SOURCE_SHA, self.builder_path, self.key.public_b64
        )
        self.assertEqual(
            lineage.read_source_sha_section(members["aos_linux_realm.wasm"]),
            SOURCE_SHA,
        )

    def test_signed_canonical_lineage_requires_exact_envelope_and_trusted_signer(self) -> None:
        output, record = self.package()
        members = validate_capsule.read_members(output)
        envelope = signed_envelope(members, self.key)
        members[lineage.PROVENANCE_MEMBER] = envelope
        canonical_archive(output, members)
        verified = json.loads(envelope)
        record["canonical_provenance"] = {"state": "signed", **verified}
        record["capsule"] = {
            "bytes": output.stat().st_size,
            "blake3": lineage.blake3_file(output),
            "sha256": lineage.sha256_file(output),
        }
        record["members"][lineage.PROVENANCE_MEMBER] = {
            "bytes": len(envelope),
            "blake3": lineage.blake3_bytes(envelope),
            "sha256": lineage.sha256_bytes(envelope),
        }
        lineage_path = output.with_suffix(output.suffix + ".lineage.json")

        def validate_record(expected_signer: str | None) -> None:
            lineage_path.write_text(json.dumps(record), encoding="utf-8")
            validate_capsule.validate(
                output, lineage_path, SOURCE_SHA, self.builder_path,
                self.key.public_b64, expected_signer,
            )

        validate_record(self.key.public_b64)
        with self.assertRaisesRegex(lineage.LineageError, "trusted signer pin"):
            validate_record(None)
        record["canonical_provenance"]["signature"] = ("A" * 86) + "=="
        with self.assertRaisesRegex(lineage.LineageError, "differs from the lineage record"):
            validate_record(self.key.public_b64)
        del record["canonical_provenance"]["signature"]
        with self.assertRaisesRegex(lineage.LineageError, "differs from the lineage record"):
            validate_record(self.key.public_b64)

    def test_local_asset_symlink_is_not_a_canonical_source(self) -> None:
        external = self.root / "external-worker.wasm"
        external.write_bytes(self.worker)
        local = self.root / "assets/worker.wasm"
        local.unlink()
        local.symlink_to(external)
        with self.assertRaisesRegex(lineage.LineageError, "contains a symlink"):
            self.package()

    def test_manifest_symlink_is_not_a_canonical_source(self) -> None:
        manifest = self.root / "Capsule.toml"
        external = self.root / "external-manifest.toml"
        external.write_bytes(manifest.read_bytes())
        manifest.unlink()
        manifest.symlink_to(external)
        with self.assertRaisesRegex(lineage.LineageError, "contains a symlink"):
            self.package()

    def test_lock_symlink_is_not_a_canonical_source(self) -> None:
        lock = self.root / "assets/linux-vcpu.lock"
        external = self.root / "external-lock"
        external.write_bytes(lock.read_bytes())
        lock.unlink()
        lock.symlink_to(external)
        with self.assertRaisesRegex(lineage.LineageError, "contains a symlink"):
            self.package()

    def test_asset_parent_directory_symlink_is_not_a_canonical_source(self) -> None:
        assets = self.root / "assets"
        external = self.root / "other-assets"
        assets.rename(external)
        assets.symlink_to(external, target_is_directory=True)
        with self.assertRaisesRegex(lineage.LineageError, "contains a symlink"):
            self.package()

    def test_builder_content_tamper_fails_before_canonicalization(self) -> None:
        self.builder_members["assets/worker.wasm"] = b"tampered worker"
        builder_archive(self.builder_path, self.builder_members)
        with self.assertRaises(lineage.LineageError) as raised:
            self.package()
        self.assertIn("digest mismatch", str(raised.exception))

    def test_builder_signature_tamper_fails_closed(self) -> None:
        envelope = json.loads(self.builder_members[lineage.PROVENANCE_MEMBER])
        envelope["signature"] = ("A" * 86) + "=="
        self.builder_members[lineage.PROVENANCE_MEMBER] = json.dumps(envelope).encode()
        builder_archive(self.builder_path, self.builder_members)
        with self.assertRaises(lineage.LineageError) as raised:
            self.package()
        self.assertIn("signature verification failed", str(raised.exception))

    def test_unsigned_output_requires_an_explicit_flag(self) -> None:
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPTS / "canonical_package.py"),
                "--builder",
                str(self.builder_path),
                "--builder-signer",
                self.key.public_b64,
            ],
            capture_output=True,
            text=True,
            check=False,
            env={**os.environ, "AOS_SOURCE_SHA": SOURCE_SHA},
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("pass --allow-unsigned only for the QA artifact", result.stderr)

    def test_valid_signature_cannot_replace_pinned_builder_or_local_asset(self) -> None:
        (self.root / "other").mkdir()
        other_key = TestKey(self.root / "other")
        unsigned = {
            name: data for name, data in self.builder_members.items()
            if name != lineage.PROVENANCE_MEMBER
        }
        unsigned[lineage.PROVENANCE_MEMBER] = signed_envelope(unsigned, other_key)
        builder_archive(self.builder_path, unsigned)
        with self.assertRaisesRegex(lineage.LineageError, "trusted signer pin"):
            self.package()

        self.resign_builder()
        self.builder_members["assets/worker.wasm"] = b"different signed worker"
        self.resign_builder()
        with self.assertRaisesRegex(lineage.LineageError, "differs from pinned source"):
            self.package()

    def test_rejects_unsafe_builder_path_and_oversized_envelope(self) -> None:
        unsafe = dict(self.builder_members)
        unsafe["../escape"] = b"bad"
        builder_archive(self.builder_path, unsafe)
        with self.assertRaisesRegex(lineage.LineageError, "non-canonical"):
            self.package()

        self.resign_builder()
        self.builder_members[lineage.PROVENANCE_MEMBER] = b"{" + b" " * (64 * 1024)
        builder_archive(self.builder_path, self.builder_members)
        with self.assertRaisesRegex(lineage.LineageError, "64 KiB"):
            self.package()

    def test_builder_pax_metadata_is_rejected(self) -> None:
        with self.builder_path.open("wb") as raw:
            with gzip.GzipFile(filename="", mode="wb", mtime=0, fileobj=raw) as compressed:
                with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                    for name, data in sorted(self.builder_members.items()):
                        info = tarfile.TarInfo(name)
                        info.size = len(data)
                        info.mode = 0o644
                        if name == "Capsule.toml":
                            info.pax_headers = {"comment": "untrusted metadata"}
                        archive.addfile(info, io.BytesIO(data))
        with self.assertRaisesRegex(lineage.LineageError, "PAX metadata"):
            self.package()

    def test_validator_rechecks_builder_instead_of_trusting_lineage_shape(self) -> None:
        output, record = self.package()
        lineage_path = output.with_suffix(output.suffix + ".lineage.json")
        record["builder_provenance"]["signature"] = "A" * 86 + "=="
        lineage_path.write_text(json.dumps(record))
        with self.assertRaisesRegex(lineage.LineageError, "differs from the verified"):
            validate_capsule.validate(
                output, lineage_path, SOURCE_SHA, self.builder_path, self.key.public_b64
            )

    def test_package_api_refuses_unsigned_output_without_flag(self) -> None:
        with self.assertRaisesRegex(lineage.LineageError, "refusing to emit"):
            canonical_package.package(
                self.builder_path,
                self.root / "dist/refused.capsule",
                allow_unsigned=False,
                expected_builder_signer=self.key.public_b64,
            )


if __name__ == "__main__":
    unittest.main()
