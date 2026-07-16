#!/usr/bin/env python3
"""Regression tests for strict signed release and channel metadata."""

from __future__ import annotations

import copy
import datetime as dt
import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("release_metadata.py")
SPEC = importlib.util.spec_from_file_location("release_metadata", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load release metadata module")
METADATA = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(METADATA)


def release_fixture() -> dict[str, object]:
    version = "2026.1.0"
    targets = {}
    for index, target in enumerate(METADATA.TARGETS, 1):
        asset = f"unicity-aos-{version}-{target}.tar.gz"
        targets[target] = {
            "asset": asset,
            "sha256": f"{index:064x}",
            "blake3": f"{index + 10:064x}",
            "sigstore-bundle": f"{asset}.sigstore.json",
            "size": index,
        }
    return {
        "schema-version": 1,
        "kind": "aos-release",
        "product": "unicity-aos-ce",
        "version": version,
        "tag": version,
        "source-commit": "a" * 40,
        "published-at": "2026-07-16T10:00:00Z",
        "release-workflow-identity": (
            "https://github.com/unicity-aos/aos-ce/.github/workflows/"
            "release.yml@refs/tags/2026.1.0"
        ),
        "runtime": {
            "repository": "astrid-runtime/astrid",
            "version": "0.9.4",
            "tag": "v0.9.4",
            "release-workflow-identity": (
                "https://github.com/unicity-astrid/astrid/.github/workflows/"
                "release.yml@refs/tags/v0.9.4"
            ),
            "release-metadata-available": False,
            "source-commit": "",
            "release-metadata-asset": "",
            "release-metadata-blake3": "",
        },
        "contracts": {
            "repository": "astrid-runtime/wit",
            "commit": "b" * 40,
            "sdk-rust-version": "0.7.1",
            "sdk-rust-commit": "c" * 40,
        },
        "gates": {
            "release-ready": False,
            "upgrade-self-heal-ready": False,
        },
        "targets": targets,
    }


def channel_fixture() -> dict[str, object]:
    release = release_fixture()
    return {
        "schema-version": 1,
        "kind": "aos-channel",
        "product": "unicity-aos-ce",
        "channel": "stable",
        "generation": 7,
        "published-at": "2026-07-16T10:00:00Z",
        "expires-at": "2026-08-15T10:00:00Z",
        "release": {
            "repository": "unicity-aos/aos-ce",
            "version": "2026.1.0",
            "tag": "2026.1.0",
            "source-commit": "a" * 40,
            "metadata-asset": "unicity-aos-2026.1.0-release.toml",
            "metadata-sha256": "d" * 64,
            "release-workflow-identity": release["release-workflow-identity"],
        },
        "targets": release["targets"],
    }


class ReleaseMetadataTests(unittest.TestCase):
    def test_calendar_semver_uses_year_plus_unbounded_semver_minor(self) -> None:
        self.assertIsNotNone(METADATA.VERSION.fullmatch("2026.13.0"))
        self.assertIsNone(METADATA.VERSION.fullmatch("2026.01.0"))
        self.assertIsNone(METADATA.VERSION.fullmatch("2025.9.0"))

    def test_release_accepts_false_staged_gates(self) -> None:
        self.assertEqual(METADATA.validate_release(release_fixture())["version"], "2026.1.0")

    def test_release_ready_mode_rejects_false_gate(self) -> None:
        with self.assertRaisesRegex(ValueError, "release-ready gate is false"):
            METADATA.validate_release(release_fixture(), require_ready=True)

    def test_release_rejects_unknown_key(self) -> None:
        fixture = release_fixture()
        fixture["surprise"] = True
        with self.assertRaisesRegex(ValueError, "unknown keys: surprise"):
            METADATA.validate_release(fixture)

    def test_release_rejects_boolean_schema_version(self) -> None:
        fixture = release_fixture()
        fixture["schema-version"] = True
        with self.assertRaisesRegex(ValueError, "integer 1"):
            METADATA.validate_release(fixture)

    def test_release_rejects_non_exact_workflow_identity(self) -> None:
        fixture = release_fixture()
        fixture["release-workflow-identity"] = (
            "https://github.com/unicity-aos/aos-ce/.github/workflows/"
            "release.yml@refs/heads/main"
        )
        with self.assertRaisesRegex(ValueError, "exact tag"):
            METADATA.validate_release(fixture)

    def test_release_rejects_ambiguous_asset_name(self) -> None:
        fixture = release_fixture()
        fixture["targets"]["aarch64-apple-darwin"]["asset"] = (
            "unicity-aos-aarch64-apple-darwin.tar.gz"
        )
        with self.assertRaisesRegex(ValueError, "must be unicity-aos-2026.1.0"):
            METADATA.validate_release(fixture)

    def test_release_rejects_unapproved_runtime_repository(self) -> None:
        fixture = release_fixture()
        fixture["runtime"]["repository"] = "example/astrid"
        with self.assertRaisesRegex(ValueError, "runtime repository"):
            METADATA.validate_release(fixture)

    def test_release_rejects_unapproved_runtime_identity(self) -> None:
        fixture = release_fixture()
        fixture["runtime"]["release-workflow-identity"] = (
            "https://github.com/example/astrid/.github/workflows/"
            "release.yml@refs/tags/v0.9.4"
        )
        with self.assertRaisesRegex(ValueError, "approved exact tag"):
            METADATA.validate_release(fixture)

    def test_release_rejects_unapproved_contract_repository(self) -> None:
        fixture = release_fixture()
        fixture["contracts"]["repository"] = "example/wit"
        with self.assertRaisesRegex(ValueError, "contracts repository"):
            METADATA.validate_release(fixture)


class ChannelMetadataTests(unittest.TestCase):
    def test_channel_accepts_expected_generation(self) -> None:
        result = METADATA.validate_channel(
            channel_fixture(),
            expected_channel="stable",
            minimum_generation=7,
            now=dt.datetime(2026, 7, 17, tzinfo=dt.timezone.utc),
        )
        self.assertEqual(result["generation"], 7)

    def test_channel_rejects_cross_channel_substitution(self) -> None:
        with self.assertRaisesRegex(ValueError, "expected dev"):
            METADATA.validate_channel(channel_fixture(), expected_channel="dev")

    def test_channel_rejects_generation_downgrade(self) -> None:
        with self.assertRaisesRegex(ValueError, "older than"):
            METADATA.validate_channel(channel_fixture(), minimum_generation=8)

    def test_channel_rejects_float_schema_version(self) -> None:
        fixture = channel_fixture()
        fixture["schema-version"] = 1.0
        with self.assertRaisesRegex(ValueError, "integer 1"):
            METADATA.validate_channel(fixture)

    def test_channel_rejects_boolean_generation(self) -> None:
        fixture = channel_fixture()
        fixture["generation"] = True
        with self.assertRaisesRegex(ValueError, "generation must be between"):
            METADATA.validate_channel(fixture)

    def test_channel_rejects_generation_larger_than_shell_consumers_support(self) -> None:
        fixture = channel_fixture()
        fixture["generation"] = METADATA.MAX_GENERATION + 1
        with self.assertRaisesRegex(ValueError, "generation must be between"):
            METADATA.validate_channel(fixture)

    def test_channel_rejects_float_target_size(self) -> None:
        fixture = channel_fixture()
        fixture["targets"]["x86_64-unknown-linux-gnu"]["size"] = 1.0
        with self.assertRaisesRegex(ValueError, "size must be positive"):
            METADATA.validate_channel(fixture)

    def test_channel_rejects_expiry(self) -> None:
        with self.assertRaisesRegex(ValueError, "expired"):
            METADATA.validate_channel(
                channel_fixture(),
                now=dt.datetime(2026, 8, 16, tzinfo=dt.timezone.utc),
            )

    def test_channel_rejects_excessive_lifetime(self) -> None:
        fixture = channel_fixture()
        fixture["expires-at"] = "2026-08-16T10:00:01Z"
        with self.assertRaisesRegex(ValueError, "lifetime exceeds"):
            METADATA.validate_channel(fixture)

    def test_channel_rejects_unreasonable_future_publication(self) -> None:
        fixture = channel_fixture()
        with self.assertRaisesRegex(ValueError, "far in the future"):
            METADATA.validate_channel(
                fixture,
                now=dt.datetime(2026, 7, 16, 9, 54, 59, tzinfo=dt.timezone.utc),
            )

    def test_channel_rejects_unknown_target(self) -> None:
        fixture = channel_fixture()
        fixture["targets"]["powerpc-unknown-linux-gnu"] = copy.deepcopy(
            fixture["targets"]["x86_64-unknown-linux-gnu"]
        )
        with self.assertRaisesRegex(ValueError, "unknown keys"):
            METADATA.validate_channel(fixture)

    def test_channel_rejects_release_digest_shape(self) -> None:
        fixture = channel_fixture()
        fixture["release"]["metadata-sha256"] = "A" * 64
        with self.assertRaisesRegex(ValueError, "SHA-256 is malformed"):
            METADATA.validate_channel(fixture)


class RenderTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def test_render_channel_copies_exact_release_targets_and_digest(self) -> None:
        release_path = self.root / "release.toml"
        release = release_fixture()
        lines = [
            "schema-version = 1",
            'kind = "aos-release"',
            'product = "unicity-aos-ce"',
            'version = "2026.1.0"',
            'tag = "2026.1.0"',
            f'source-commit = "{release["source-commit"]}"',
            'published-at = "2026-07-16T10:00:00Z"',
            f'release-workflow-identity = "{release["release-workflow-identity"]}"',
            "",
            "[runtime]",
            'repository = "astrid-runtime/astrid"',
            'version = "0.9.4"',
            'tag = "v0.9.4"',
            f'release-workflow-identity = "{release["runtime"]["release-workflow-identity"]}"',
            "release-metadata-available = false",
            'source-commit = ""',
            'release-metadata-asset = ""',
            'release-metadata-blake3 = ""',
            "",
            "[contracts]",
            'repository = "astrid-runtime/wit"',
            f'commit = "{release["contracts"]["commit"]}"',
            'sdk-rust-version = "0.7.1"',
            f'sdk-rust-commit = "{release["contracts"]["sdk-rust-commit"]}"',
            "",
            "[gates]",
            "release-ready = false",
            "upgrade-self-heal-ready = false",
        ]
        METADATA.write_target_tables(lines, release["targets"])
        release_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
        output = self.root / "stable.toml"
        args = type(
            "Args",
            (),
            {
                "channel": "stable",
                "generation": 1,
                "published_at": "2026-07-16T10:00:00Z",
                "expires_at": "2026-08-15T10:00:00Z",
                "release_metadata": release_path,
                "release_metadata_sha256": None,
                "require_ready": False,
                "output": output,
            },
        )()
        METADATA.render_channel(args)
        rendered = METADATA.load(output)
        METADATA.validate_channel(rendered, expected_channel="stable")
        self.assertEqual(rendered["targets"], release["targets"])

    def test_render_release_carries_staged_runtime_metadata_unavailability(self) -> None:
        artifacts = self.root / "artifacts"
        artifacts.mkdir()
        sha_lines = []
        blake_lines = []
        import hashlib

        for target in METADATA.TARGETS:
            asset = f"unicity-aos-2026.1.0-{target}.tar.gz"
            payload = f"fixture-{target}".encode()
            (artifacts / asset).write_bytes(payload)
            sha_lines.append(f"{hashlib.sha256(payload).hexdigest()}  {asset}")
            blake_lines.append(f"{'1' * 64}  {asset}")
        sha_path = self.root / "SHA256SUMS.txt"
        blake_path = self.root / "BLAKE3SUMS.txt"
        sha_path.write_text("\n".join(sha_lines) + "\n", encoding="utf-8")
        blake_path.write_text("\n".join(blake_lines) + "\n", encoding="utf-8")
        output = self.root / "release.toml"
        args = type(
            "Args",
            (),
            {
                "version": "2026.1.0",
                "tag": "2026.1.0",
                "source_commit": "a" * 40,
                "published_at": "2026-07-16T10:00:00Z",
                "artifacts": artifacts,
                "sha256": sha_path,
                "blake3": blake_path,
                "output": output,
            },
        )()
        METADATA.render_release(args)
        rendered = METADATA.load(output)
        self.assertFalse(rendered["runtime"]["release-metadata-available"])
        self.assertEqual(rendered["runtime"]["source-commit"], "")
        self.assertEqual(rendered["runtime"]["release-metadata-asset"], "")
        self.assertEqual(rendered["runtime"]["release-metadata-blake3"], "")


if __name__ == "__main__":
    unittest.main()
