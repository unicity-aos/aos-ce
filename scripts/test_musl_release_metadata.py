#!/usr/bin/env python3
"""Regression tests for the staged Linux musl release metadata contract."""

from __future__ import annotations

import copy
import hashlib
import tempfile
import unittest
from pathlib import Path

import musl_release_metadata as MUSL
import release_metadata
from test_release_metadata import release_fixture


VERSION = "2026.1.3"
RUNTIME_VERSION = "0.10.4"


def runtime_pin_fixture(*, release_ready: bool) -> dict[str, object]:
    runtime = {
        "repository": "astrid-runtime/astrid",
        "release-ready": release_ready,
        "version": RUNTIME_VERSION,
        "tag": f"v{RUNTIME_VERSION}",
        "release-workflow-identity": (
            "https://github.com/astrid-runtime/astrid/.github/workflows/"
            f"release.yml@refs/tags/v{RUNTIME_VERSION}"
        ),
        "source-commit": "b" * 40,
        "legacy-release-metadata-asset": f"astrid-{RUNTIME_VERSION}-release.toml",
        "legacy-release-metadata-blake3": "c" * 64,
        "musl-release-metadata-asset": (
            f"astrid-{RUNTIME_VERSION}-musl-release.toml"
            if release_ready
            else ""
        ),
        "musl-release-metadata-blake3": "d" * 64 if release_ready else "",
    }
    return {"schema-version": 1, "runtime": runtime}


def extension_fixture() -> tuple[dict[str, object], dict[str, object], bytes]:
    legacy = release_fixture()
    legacy_bytes = b"fixture legacy release metadata"
    targets = {}
    for index, target in enumerate(MUSL.MUSL_TARGETS, 1):
        asset = f"unicity-aos-{VERSION}-{target}.tar.gz"
        targets[target] = {
            "asset": asset,
            "sha256": f"{index:064x}",
            "blake3": f"{index + 10:064x}",
            "sigstore-bundle": f"{asset}.sigstore.json",
            "size": index,
        }
    runtime = copy.deepcopy(runtime_pin_fixture(release_ready=True)["runtime"])
    runtime.pop("release-ready")
    extension = {
        "schema-version": 1,
        "kind": MUSL.KIND,
        "product": release_metadata.PRODUCT,
        "repository": release_metadata.REPOSITORY,
        "version": VERSION,
        "tag": VERSION,
        "source-commit": legacy["source-commit"],
        "release-workflow-identity": legacy["release-workflow-identity"],
        "legacy-release": {
            "metadata-asset": f"unicity-aos-{VERSION}-release.toml",
            "metadata-sha256": hashlib.sha256(legacy_bytes).hexdigest(),
        },
        "runtime-musl": runtime,
        "targets": targets,
    }
    return extension, legacy, legacy_bytes


class RuntimePinTests(unittest.TestCase):
    def test_unready_pin_is_admitted_but_require_ready_rejects_it(self) -> None:
        pin = runtime_pin_fixture(release_ready=False)
        runtime = MUSL.validate_runtime_pin(pin, require_ready=False)
        self.assertFalse(runtime["release-ready"])
        with self.assertRaisesRegex(ValueError, "release-ready gate is false"):
            MUSL.validate_runtime_pin(pin, require_ready=True)

    def test_rejects_unknown_keys(self) -> None:
        pin = runtime_pin_fixture(release_ready=False)
        pin["runtime"]["surprise"] = True
        with self.assertRaisesRegex(ValueError, "unknown keys: surprise"):
            MUSL.validate_runtime_pin(pin, require_ready=False)

    def test_rejects_tag_version_mismatch(self) -> None:
        pin = runtime_pin_fixture(release_ready=False)
        pin["runtime"]["tag"] = "v9.9.9"
        with self.assertRaisesRegex(ValueError, "tag/version mismatch"):
            MUSL.validate_runtime_pin(pin, require_ready=False)

    def test_rejects_malformed_digests(self) -> None:
        pin = runtime_pin_fixture(release_ready=False)
        pin["runtime"]["legacy-release-metadata-blake3"] = "bad"
        with self.assertRaisesRegex(ValueError, "legacy metadata BLAKE3"):
            MUSL.validate_runtime_pin(pin, require_ready=False)

        ready = runtime_pin_fixture(release_ready=True)
        ready["runtime"]["musl-release-metadata-blake3"] = "bad"
        with self.assertRaisesRegex(ValueError, "extension metadata BLAKE3"):
            MUSL.validate_runtime_pin(ready, require_ready=False)

    def test_rejects_non_empty_musl_fields_when_unready(self) -> None:
        pin = runtime_pin_fixture(release_ready=False)
        pin["runtime"]["musl-release-metadata-asset"] = "pending.toml"
        with self.assertRaisesRegex(ValueError, "must remain empty"):
            MUSL.validate_runtime_pin(pin, require_ready=False)

    def test_rejects_empty_musl_fields_when_ready(self) -> None:
        pin = runtime_pin_fixture(release_ready=True)
        pin["runtime"]["musl-release-metadata-asset"] = ""
        with self.assertRaises(ValueError):
            MUSL.validate_runtime_pin(pin, require_ready=False)


class ExtensionTests(unittest.TestCase):
    def test_round_trip_binds_exactly_two_targets_to_legacy_release(self) -> None:
        extension, legacy, legacy_bytes = extension_fixture()
        MUSL.validate_extension(
            extension,
            legacy=legacy,
            legacy_bytes=legacy_bytes,
        )
        self.assertEqual(set(extension["targets"]), set(MUSL.MUSL_TARGETS))

        rendered = MUSL.render_extension(extension)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "extension.toml"
            path.write_text(rendered, encoding="utf-8")
            loaded = release_metadata.load(path)
        MUSL.validate_extension(loaded, legacy=legacy, legacy_bytes=legacy_bytes)
        self.assertEqual(loaded, extension)

    def test_rejects_missing_unknown_and_duplicate_target_data(self) -> None:
        extension, _, _ = extension_fixture()
        missing = copy.deepcopy(extension)
        missing["targets"].pop(MUSL.MUSL_TARGETS[0])
        with self.assertRaisesRegex(ValueError, "missing keys"):
            MUSL.validate_extension(missing)

        unknown = copy.deepcopy(extension)
        unknown["targets"]["x86_64-unknown-linux-gnu"] = copy.deepcopy(
            unknown["targets"][MUSL.MUSL_TARGETS[1]]
        )
        with self.assertRaisesRegex(ValueError, "unknown keys"):
            MUSL.validate_extension(unknown)

        duplicate = copy.deepcopy(extension)
        duplicate["targets"][MUSL.MUSL_TARGETS[0]] = copy.deepcopy(
            duplicate["targets"][MUSL.MUSL_TARGETS[1]]
        )
        with self.assertRaisesRegex(ValueError, "asset must be"):
            MUSL.validate_extension(duplicate)

    def test_rejects_legacy_identity_mismatch(self) -> None:
        extension, legacy, legacy_bytes = extension_fixture()
        mismatched_legacy = copy.deepcopy(legacy)
        mismatched_legacy["source-commit"] = "c" * 40
        with self.assertRaisesRegex(ValueError, "source-commit differs"):
            MUSL.validate_extension(
                extension,
                legacy=mismatched_legacy,
                legacy_bytes=legacy_bytes,
            )

    def test_rejects_legacy_digest_mismatch(self) -> None:
        extension, legacy, legacy_bytes = extension_fixture()
        extension["legacy-release"]["metadata-sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "bind the authenticated"):
            MUSL.validate_extension(
                extension,
                legacy=legacy,
                legacy_bytes=legacy_bytes,
            )

    def test_rejects_runtime_identity_and_unknown_keys(self) -> None:
        extension, _, _ = extension_fixture()
        extension["runtime-musl"]["tag"] = "v9.9.9"
        with self.assertRaisesRegex(ValueError, "tag/version mismatch"):
            MUSL.validate_extension(extension)

        extension, _, _ = extension_fixture()
        extension["runtime-musl"]["surprise"] = True
        with self.assertRaisesRegex(ValueError, "unknown keys"):
            MUSL.validate_extension(extension)

    def test_checked_in_pin_is_staged_and_fail_closed(self) -> None:
        path = Path(__file__).resolve().parent.parent / "release/runtime-musl-compatibility.toml"
        pin = release_metadata.load(path)
        runtime = MUSL.validate_runtime_pin(pin, require_ready=False)
        self.assertFalse(runtime["release-ready"])
        with self.assertRaisesRegex(ValueError, "release-ready gate is false"):
            MUSL.validate_runtime_pin(pin, require_ready=True)


if __name__ == "__main__":
    unittest.main()
