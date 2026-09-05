#!/usr/bin/env python3

from __future__ import annotations

import io
import json
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from typing import Optional

sys.path.insert(0, str(Path(__file__).resolve().parent))

from capsule_release import ContractError, CapsuleSpec, source_contract, validate_artifacts


EXPECTED_CAPSULE_ASSETS = frozenset(
    {
        "aos-cli.capsule",
        "aos-mcp.capsule",
        "aos-registry.capsule",
        "aos-openai-compat.capsule",
        "aos-react.capsule",
        "aos-session.capsule",
        "aos-identity.capsule",
        "aos-users.capsule",
        "aos-router.capsule",
        "aos-prompt-builder.capsule",
        "aos-context-engine.capsule",
        "aos-hook-bridge.capsule",
        "aos-hook-adapter-oracle.capsule",
        "aos-meta-harness.capsule",
        "aos-shell.capsule",
        "aos-http.capsule",
        "aos-fs.capsule",
        "aos-system.capsule",
        "aos-forge.capsule",
        "aos-skills.capsule",
        "aos-agents.capsule",
        "aos-memory.capsule",
    }
)


def add_bytes(
    archive: tarfile.TarFile,
    name: str,
    data: bytes,
    *,
    kind: Optional[bytes] = None,
) -> None:
    member = tarfile.TarInfo(name)
    member.size = len(data)
    member.mtime = 0
    member.uid = 0
    member.gid = 0
    if kind is not None:
        member.type = kind
        member.size = 0
    archive.addfile(member, io.BytesIO(data) if member.isfile() else None)


def provenance_bytes(*, mutation: Optional[str] = None) -> bytes:
    envelope: dict[str, object] = {
        "schema_version": 1,
        "algorithm": "ed25519-blake3-tree-v1",
        "content_digest": "0" * 64,
        "signer": "fixture-signer",
        "signature": "fixture-signature",
    }
    if mutation == "provenance-wrong-schema":
        envelope["schema_version"] = 2
    elif mutation == "provenance-wrong-algorithm":
        envelope["algorithm"] = "wrong-algorithm"
    elif mutation == "provenance-wrong-digest":
        envelope["content_digest"] = "not-a-digest"
    elif mutation == "provenance-extra-key":
        envelope["extra"] = "not allowed"
    return json.dumps(envelope).encode("utf-8")


def write_fixture(path: Path, spec: CapsuleSpec, *, mutation: Optional[str] = None) -> None:
    manifest = spec.manifest.read_bytes()
    with tarfile.open(path, mode="w:gz") as archive:
        if mutation == "traversal":
            add_bytes(archive, "../escape", b"bad")
        if mutation == "duplicate-manifest":
            add_bytes(archive, "Capsule.toml", manifest)
        if mutation == "dot-alias":
            add_bytes(archive, "./Capsule.toml", b'[package]\nname = "wrong-package"\nversion = "0.0.0"\n')
        if mutation == "case-alias":
            add_bytes(archive, "capsule.toml", b"bad")
        if mutation == "symlink":
            link = tarfile.TarInfo("outside")
            link.type = tarfile.SYMTYPE
            link.linkname = "/tmp"
            archive.addfile(link)
        if mutation == "hardlink":
            link = tarfile.TarInfo("outside")
            link.type = tarfile.LNKTYPE
            link.linkname = "Capsule.toml"
            archive.addfile(link)
        if mutation == "device":
            device = tarfile.TarInfo("device")
            device.type = tarfile.CHRTYPE
            archive.addfile(device)
        if mutation == "unexpected-member":
            add_bytes(archive, "unexpected.txt", b"not allowed")
        if mutation == "provenance-directory":
            add_bytes(archive, "Capsule.provenance.json", b"", kind=tarfile.DIRTYPE)
        if mutation == "provenance-non-json":
            add_bytes(archive, "Capsule.provenance.json", b"not json")
        if mutation == "wrong-manifest":
            manifest = manifest.replace(
                f'name = "{spec.package}"'.encode(),
                b'name = "wrong-package"',
                1,
            )
        if mutation == "changed-capability":
            manifest = manifest.replace(b"uplink = true", b"uplink = false", 1)
        add_bytes(archive, "Capsule.toml", manifest)
        for component in spec.components:
            if mutation == "missing-component" and component == spec.components[0]:
                continue
            add_bytes(archive, component, b"\x00asm")
        if mutation != "missing-wit-tree":
            add_bytes(archive, "wit", b"", kind=tarfile.DIRTYPE)
            if mutation != "missing-wit-capsule":
                add_bytes(archive, "wit/capsule.wit", b"package fixture:capsule;\n")
            add_bytes(archive, "wit/deps", b"", kind=tarfile.DIRTYPE)
            add_bytes(archive, "wit/deps/astrid-contracts", b"", kind=tarfile.DIRTYPE)
            if mutation != "missing-contracts-wit":
                contracts_wit = b"package fixture:contracts;\n"
                if mutation == "empty-wit":
                    contracts_wit = b""
                add_bytes(
                    archive,
                    "wit/deps/astrid-contracts/astrid-contracts.wit",
                    contracts_wit,
                )
        if mutation != "missing-provenance":
            if mutation == "provenance-directory":
                pass
            elif mutation == "provenance-non-json":
                pass
            else:
                add_bytes(archive, "Capsule.provenance.json", provenance_bytes(mutation=mutation))


class CapsuleReleaseTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.specs = source_contract()

    def fixture_set(self, directory: Path) -> None:
        for spec in self.specs:
            write_fixture(directory / spec.asset, spec)

    def test_source_contract_has_exact_community_set(self) -> None:
        self.assertEqual(len(self.specs), 22)
        self.assertEqual(len({spec.asset for spec in self.specs}), 22)
        assets = {spec.asset for spec in self.specs}
        self.assertEqual(assets, EXPECTED_CAPSULE_ASSETS)
        distro = Path(__file__).resolve().parent.parent / "distros/community/unicity-ce/Distro.toml"
        text = distro.read_text(encoding="utf-8")
        self.assertNotIn("@unicity-aos/", text)
        self.assertEqual(text.count('source = "capsules/'), 22)
        for asset in EXPECTED_CAPSULE_ASSETS:
            self.assertIn(f'source = "capsules/{asset}"', text)

    def test_accepts_exact_safe_artifact_set(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            self.fixture_set(directory)
            validate_artifacts(directory, self.specs)

    def assert_mutation_rejected(self, mutation: str) -> None:
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            self.fixture_set(directory)
            write_fixture(directory / self.specs[0].asset, self.specs[0], mutation=mutation)
            with self.assertRaises(ContractError):
                validate_artifacts(directory, self.specs)

    def test_rejects_traversal(self) -> None:
        self.assert_mutation_rejected("traversal")

    def test_rejects_exact_duplicate(self) -> None:
        self.assert_mutation_rejected("duplicate-manifest")

    def test_rejects_dot_alias(self) -> None:
        self.assert_mutation_rejected("dot-alias")

    def test_rejects_case_alias(self) -> None:
        self.assert_mutation_rejected("case-alias")

    def test_rejects_symlink(self) -> None:
        self.assert_mutation_rejected("symlink")

    def test_rejects_hardlink(self) -> None:
        self.assert_mutation_rejected("hardlink")

    def test_rejects_device(self) -> None:
        self.assert_mutation_rejected("device")

    def test_rejects_unexpected_member(self) -> None:
        self.assert_mutation_rejected("unexpected-member")

    def test_rejects_missing_provenance(self) -> None:
        self.assert_mutation_rejected("missing-provenance")

    def test_rejects_missing_wit_tree(self) -> None:
        self.assert_mutation_rejected("missing-wit-tree")

    def test_rejects_missing_wit_capsule(self) -> None:
        self.assert_mutation_rejected("missing-wit-capsule")

    def test_rejects_missing_contracts_wit(self) -> None:
        self.assert_mutation_rejected("missing-contracts-wit")

    def test_rejects_provenance_directory(self) -> None:
        self.assert_mutation_rejected("provenance-directory")

    def test_rejects_non_json_provenance(self) -> None:
        self.assert_mutation_rejected("provenance-non-json")

    def test_rejects_wrong_provenance_schema(self) -> None:
        self.assert_mutation_rejected("provenance-wrong-schema")

    def test_rejects_wrong_provenance_algorithm(self) -> None:
        self.assert_mutation_rejected("provenance-wrong-algorithm")

    def test_rejects_wrong_provenance_digest(self) -> None:
        self.assert_mutation_rejected("provenance-wrong-digest")

    def test_rejects_extra_provenance_key(self) -> None:
        self.assert_mutation_rejected("provenance-extra-key")

    def test_rejects_empty_wit(self) -> None:
        self.assert_mutation_rejected("empty-wit")

    def test_rejects_wrong_embedded_identity(self) -> None:
        self.assert_mutation_rejected("wrong-manifest")

    def test_rejects_changed_capabilities(self) -> None:
        self.assert_mutation_rejected("changed-capability")

    def test_rejects_missing_component(self) -> None:
        self.assert_mutation_rejected("missing-component")

    def test_rejects_unexpected_asset(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            self.fixture_set(directory)
            (directory / "unexpected.capsule").write_bytes(b"no")
            with self.assertRaises(ContractError):
                validate_artifacts(directory, self.specs)

    def test_rejects_unexpected_directory(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            self.fixture_set(directory)
            (directory / "unexpected").mkdir()
            with self.assertRaises(ContractError):
                validate_artifacts(directory, self.specs)

    def test_rejects_symlink_asset(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            self.fixture_set(directory)
            target = directory / self.specs[0].asset
            target.unlink()
            target.symlink_to(directory / self.specs[1].asset)
            with self.assertRaises(ContractError):
                validate_artifacts(directory, self.specs)


if __name__ == "__main__":
    unittest.main()
