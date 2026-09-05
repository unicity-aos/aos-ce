#!/usr/bin/env python3

from __future__ import annotations

import argparse
import hashlib
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import capsule_release
import release_metadata
import release_publication
from test_capsule_release import write_fixture


VERSION = "2026.9.0"
SOURCE_COMMIT = "a" * 40
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


class ReleasePublicationTests(unittest.TestCase):
    def fixture(self, root: Path) -> tuple[Path, Path]:
        artifacts = root / "artifacts"
        artifacts.mkdir()
        specs = capsule_release.source_contract()
        for spec in specs:
            write_fixture(artifacts / spec.asset, spec)

        for target in release_metadata.TARGETS:
            (artifacts / f"unicity-aos-{VERSION}-{target}.tar.gz").write_bytes(
                f"archive:{target}".encode()
            )

        checksummed = sorted(
            path.name
            for path in artifacts.iterdir()
            if path.name.endswith((".tar.gz", ".capsule"))
        )
        sha_lines = []
        blake_lines = []
        for name in checksummed:
            value = (artifacts / name).read_bytes()
            sha_lines.append(f"{hashlib.sha256(value).hexdigest()}  {name}")
            blake_lines.append(f"{hashlib.sha256(b'blake3:' + value).hexdigest()}  {name}")
        (artifacts / "SHA256SUMS.txt").write_text("\n".join(sha_lines) + "\n")
        (artifacts / "BLAKE3SUMS.txt").write_text("\n".join(blake_lines) + "\n")

        compatibility = root / "runtime-compatibility.toml"
        compatibility.write_text(
            (release_publication.ROOT / "release" / "runtime-compatibility.toml")
            .read_text()
            .replace("release-ready = false", "release-ready = true")
            .replace("upgrade-self-heal-ready = false", "upgrade-self-heal-ready = true")
        )
        shutil.copyfile(compatibility, artifacts / "runtime-compatibility.toml")

        metadata = artifacts / f"unicity-aos-{VERSION}-release.toml"
        args = argparse.Namespace(
            version=VERSION,
            tag=VERSION,
            source_commit=SOURCE_COMMIT,
            published_at="2026-07-16T00:00:00Z",
            artifacts=artifacts,
            sha256=artifacts / "SHA256SUMS.txt",
            blake3=artifacts / "BLAKE3SUMS.txt",
            output=metadata,
        )
        release_metadata.render_release(args)
        metadata.write_text(
            metadata.read_text()
            .replace("release-ready = false", "release-ready = true")
            .replace("upgrade-self-heal-ready = false", "upgrade-self-heal-ready = true")
        )

        payloads = [
            *checksummed,
            "BLAKE3SUMS.txt",
            "SHA256SUMS.txt",
            "runtime-compatibility.toml",
            metadata.name,
        ]
        for name in payloads:
            (artifacts / f"{name}.sigstore.json").write_text("{}\n")
        return artifacts, compatibility

    def validate(self, artifacts: Path, compatibility: Path) -> list[str]:
        return release_publication.validate_release_assets(
            artifacts,
            version=VERSION,
            source_commit=SOURCE_COMMIT,
            compatibility_path=compatibility,
        )

    def test_accepts_complete_authenticated_inventory(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifacts, compatibility = self.fixture(Path(temp))
            payloads = self.validate(artifacts, compatibility)
            self.assertIn(f"unicity-aos-{VERSION}-release.toml", payloads)
            capsules = {name for name in payloads if name.endswith(".capsule")}
            self.assertEqual(capsules, EXPECTED_CAPSULE_ASSETS)
            self.assertTrue(
                {
                    "aos-mcp.capsule",
                    "aos-hook-adapter-oracle.capsule",
                    "aos-meta-harness.capsule",
                }
                <= capsules
            )

    def test_rejects_missing_named_capsule_or_bundle(self) -> None:
        for asset in EXPECTED_CAPSULE_ASSETS:
            with self.subTest(asset=asset):
                with tempfile.TemporaryDirectory() as temp:
                    artifacts, compatibility = self.fixture(Path(temp))
                    (artifacts / asset).unlink()
                    with self.assertRaisesRegex(ValueError, "asset set differs"):
                        self.validate(artifacts, compatibility)

                with tempfile.TemporaryDirectory() as temp:
                    artifacts, compatibility = self.fixture(Path(temp))
                    (artifacts / f"{asset}.sigstore.json").unlink()
                    with self.assertRaisesRegex(ValueError, "asset set differs"):
                        self.validate(artifacts, compatibility)

    def test_rejects_missing_asset(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifacts, compatibility = self.fixture(Path(temp))
            next(artifacts.glob("*.capsule.sigstore.json")).unlink()
            with self.assertRaisesRegex(ValueError, "asset set differs"):
                self.validate(artifacts, compatibility)

    def test_rejects_unexpected_asset(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifacts, compatibility = self.fixture(Path(temp))
            (artifacts / "unexpected").write_text("no")
            with self.assertRaisesRegex(ValueError, "asset set differs"):
                self.validate(artifacts, compatibility)

    def test_rejects_changed_payload(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifacts, compatibility = self.fixture(Path(temp))
            next(artifacts.glob("*.tar.gz")).write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
                self.validate(artifacts, compatibility)

    def test_rejects_wrong_source_commit(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifacts, compatibility = self.fixture(Path(temp))
            with self.assertRaisesRegex(ValueError, "source commit"):
                release_publication.validate_release_assets(
                    artifacts,
                    version=VERSION,
                    source_commit="b" * 40,
                    compatibility_path=compatibility,
                )

    def test_rejects_compatibility_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifacts, compatibility = self.fixture(Path(temp))
            compatibility.write_text(compatibility.read_text() + "\n# drift\n")
            with self.assertRaisesRegex(ValueError, "tagged source"):
                self.validate(artifacts, compatibility)


if __name__ == "__main__":
    unittest.main()
