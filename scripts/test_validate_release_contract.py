#!/usr/bin/env python3
"""Tests for the parsed release-readiness publication gate."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import musl_release_metadata


SCRIPT = Path(__file__).with_name("validate-release-contract.py")
ROOT = SCRIPT.parent.parent
SPEC = importlib.util.spec_from_file_location("validate_release_contract", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load release contract validator")
VALIDATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATOR)


class ReleaseReadinessTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.metadata_path = Path(self.temporary_directory.name) / "runtime.toml"

    def write_metadata(self, text: str) -> Path:
        self.metadata_path.write_text(text, encoding="utf-8")
        return self.metadata_path

    def parse(self, text: str) -> dict[str, object]:
        return VALIDATOR.readiness_metadata(self.write_metadata(text))

    def test_false_passes_staged_validation(self) -> None:
        metadata = self.parse(
            "schema-version = 1\n[runtime]\n"
            "release-ready = false\nupgrade-self-heal-ready = false\n"
        )
        self.assertFalse(
            VALIDATOR.validate_release_readiness(
                metadata, require_release_ready=False
            )
        )

    def test_false_fails_publication_validation(self) -> None:
        metadata = self.parse(
            "schema-version = 1\n[runtime]\n"
            "release-ready = false\nupgrade-self-heal-ready = false\n"
        )
        with self.assertRaisesRegex(ValueError, "refusing to publish"):
            VALIDATOR.validate_release_readiness(
                metadata, require_release_ready=True
            )

    def test_true_passes_publication_validation(self) -> None:
        metadata = self.parse(
            "schema-version = 1\n[runtime]\n"
            "release-ready = true\nupgrade-self-heal-ready = true\n"
        )
        self.assertTrue(
            VALIDATOR.validate_release_readiness(
                metadata, require_release_ready=True
            )
        )

    def test_duplicate_readiness_keys_are_invalid_toml(self) -> None:
        with self.assertRaises(VALIDATOR.tomllib.TOMLDecodeError):
            self.parse(
                "schema-version = 1\n"
                "[runtime]\n"
                "release-ready = false\n"
                "upgrade-self-heal-ready = false\n"
                "release-ready = true\n"
            )

    def test_duplicate_runtime_sections_are_invalid_toml(self) -> None:
        with self.assertRaises(VALIDATOR.tomllib.TOMLDecodeError):
            self.parse(
                "schema-version = 1\n"
                "[runtime]\n"
                "release-ready = false\n"
                "upgrade-self-heal-ready = false\n"
                "[runtime]\n"
                "release-ready = true\n"
                "upgrade-self-heal-ready = true\n"
            )

    def test_readiness_must_be_a_boolean(self) -> None:
        metadata = self.parse(
            'schema-version = 1\n[runtime]\nrelease-ready = "false"\n'
            "upgrade-self-heal-ready = false\n"
        )
        with self.assertRaisesRegex(ValueError, "must be a boolean"):
            VALIDATOR.validate_release_readiness(
                metadata, require_release_ready=False
            )

    def test_schema_version_is_required(self) -> None:
        metadata = self.parse(
            "schema-version = 2\n[runtime]\n"
            "release-ready = false\nupgrade-self-heal-ready = false\n"
        )
        with self.assertRaisesRegex(ValueError, "schema-version must be 1"):
            VALIDATOR.validate_release_readiness(
                metadata, require_release_ready=False
            )

    def test_malformed_toml_is_rejected(self) -> None:
        with self.assertRaises(VALIDATOR.tomllib.TOMLDecodeError):
            self.parse("schema-version = 1\n[runtime\nrelease-ready = false\n")

    def test_upgrade_self_heal_gate_is_required(self) -> None:
        metadata = self.parse("schema-version = 1\n[runtime]\nrelease-ready = false\n")
        with self.assertRaisesRegex(ValueError, "upgrade-self-heal-ready"):
            VALIDATOR.validate_release_readiness(
                metadata, require_release_ready=False
            )

    def test_upgrade_self_heal_gate_blocks_publication(self) -> None:
        metadata = self.parse(
            "schema-version = 1\n[runtime]\n"
            "release-ready = true\nupgrade-self-heal-ready = false\n"
        )
        with self.assertRaisesRegex(ValueError, "exact candidate"):
            VALIDATOR.validate_release_readiness(
                metadata, require_release_ready=True
            )

    def test_nightly_product_version_requires_a_real_date(self) -> None:
        valid = "2026.1.3-nightly.20260717.g" + "a" * 40
        VALIDATOR.validate_product_version(valid, allow_nightly=True)
        with self.assertRaisesRegex(ValueError, "invalid date"):
            VALIDATOR.validate_product_version(
                "2026.1.3-nightly.20260230.g" + "a" * 40,
                allow_nightly=True,
            )
        with self.assertRaisesRegex(ValueError, "allowed calendar"):
            VALIDATOR.validate_product_version(valid, allow_nightly=False)

    def test_main_matches_the_current_publication_gate(self) -> None:
        self.assertEqual(VALIDATOR.main([]), 0)
        runtime = VALIDATOR.readiness_metadata(
            ROOT / "release/runtime-compatibility.toml"
        )["runtime"]
        if runtime["release-ready"] and runtime["upgrade-self-heal-ready"]:
            self.assertEqual(VALIDATOR.main(["--require-release-ready"]), 0)
        else:
            with self.assertRaisesRegex(ValueError, "refusing to publish"):
                VALIDATOR.main(["--require-release-ready"])

    def test_main_admits_checked_in_false_ready_musl_pin(self) -> None:
        pin = VALIDATOR.readiness_metadata(
            ROOT / "release/runtime-musl-compatibility.toml"
        )
        runtime = musl_release_metadata.validate_runtime_pin(
            pin, require_ready=False
        )
        self.assertFalse(runtime["release-ready"])
        self.assertEqual(VALIDATOR.main([]), 0)

    def test_require_release_ready_follows_the_gnu_gate_only(self) -> None:
        gnu = VALIDATOR.readiness_metadata(
            ROOT / "release/runtime-compatibility.toml"
        )["runtime"]
        musl = VALIDATOR.readiness_metadata(
            ROOT / "release/runtime-musl-compatibility.toml"
        )["runtime"]
        self.assertTrue(gnu["release-ready"])
        self.assertFalse(musl["release-ready"])
        self.assertEqual(VALIDATOR.main(["--require-release-ready"]), 0)

    def test_musl_pin_rejects_identity_extra_keys_and_empty_ready_fields(self) -> None:
        pin = VALIDATOR.readiness_metadata(
            ROOT / "release/runtime-musl-compatibility.toml"
        )
        pin["runtime"]["tag"] = "v9.9.9"
        with self.assertRaisesRegex(ValueError, "tag/version mismatch"):
            musl_release_metadata.validate_runtime_pin(pin, require_ready=False)

        pin = VALIDATOR.readiness_metadata(
            ROOT / "release/runtime-musl-compatibility.toml"
        )
        pin["runtime"]["surprise"] = True
        with self.assertRaisesRegex(ValueError, "unknown keys"):
            musl_release_metadata.validate_runtime_pin(pin, require_ready=False)

        pin = VALIDATOR.readiness_metadata(
            ROOT / "release/runtime-musl-compatibility.toml"
        )
        pin["runtime"]["release-ready"] = True
        pin["runtime"]["musl-release-metadata-blake3"] = ""
        with self.assertRaises(ValueError):
            musl_release_metadata.validate_runtime_pin(pin, require_ready=False)

    def test_cli_matches_the_current_publication_gate(self) -> None:
        staged = subprocess.run(
            [sys.executable, str(SCRIPT)],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(staged.returncode, 0, staged.stderr)

        runtime = VALIDATOR.readiness_metadata(
            ROOT / "release/runtime-compatibility.toml"
        )["runtime"]
        strict = subprocess.run(
            [sys.executable, str(SCRIPT), "--require-release-ready"],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )
        if runtime["release-ready"] and runtime["upgrade-self-heal-ready"]:
            self.assertEqual(strict.returncode, 0, strict.stderr)
        else:
            self.assertEqual(strict.returncode, 1)
            self.assertIn("refusing to publish", strict.stderr)


if __name__ == "__main__":
    unittest.main()
