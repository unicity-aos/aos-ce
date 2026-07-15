#!/usr/bin/env python3
"""Tests for the parsed release-readiness publication gate."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


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
            "schema-version = 1\n[runtime]\nrelease-ready = false\n"
        )
        self.assertFalse(
            VALIDATOR.validate_release_readiness(
                metadata, require_release_ready=False
            )
        )

    def test_false_fails_publication_validation(self) -> None:
        metadata = self.parse(
            "schema-version = 1\n[runtime]\nrelease-ready = false\n"
        )
        with self.assertRaisesRegex(ValueError, "refusing to publish"):
            VALIDATOR.validate_release_readiness(
                metadata, require_release_ready=True
            )

    def test_true_passes_publication_validation(self) -> None:
        metadata = self.parse(
            "schema-version = 1\n[runtime]\nrelease-ready = true\n"
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
                "release-ready = true\n"
            )

    def test_duplicate_runtime_sections_are_invalid_toml(self) -> None:
        with self.assertRaises(VALIDATOR.tomllib.TOMLDecodeError):
            self.parse(
                "schema-version = 1\n"
                "[runtime]\n"
                "release-ready = false\n"
                "[runtime]\n"
                "release-ready = true\n"
            )

    def test_readiness_must_be_a_boolean(self) -> None:
        metadata = self.parse(
            'schema-version = 1\n[runtime]\nrelease-ready = "false"\n'
        )
        with self.assertRaisesRegex(ValueError, "must be a boolean"):
            VALIDATOR.validate_release_readiness(
                metadata, require_release_ready=False
            )

    def test_schema_version_is_required(self) -> None:
        metadata = self.parse(
            "schema-version = 2\n[runtime]\nrelease-ready = false\n"
        )
        with self.assertRaisesRegex(ValueError, "schema-version must be 1"):
            VALIDATOR.validate_release_readiness(
                metadata, require_release_ready=False
            )

    def test_malformed_toml_is_rejected(self) -> None:
        with self.assertRaises(VALIDATOR.tomllib.TOMLDecodeError):
            self.parse("schema-version = 1\n[runtime\nrelease-ready = false\n")

    def test_main_passes_staged_and_refuses_strict_validation(self) -> None:
        self.assertEqual(VALIDATOR.main([]), 0)
        with self.assertRaisesRegex(ValueError, "refusing to publish"):
            VALIDATOR.main(["--require-release-ready"])

    def test_cli_passes_staged_and_refuses_strict_validation(self) -> None:
        staged = subprocess.run(
            [sys.executable, str(SCRIPT)],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(staged.returncode, 0, staged.stderr)

        strict = subprocess.run(
            [sys.executable, str(SCRIPT), "--require-release-ready"],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(strict.returncode, 1)
        self.assertIn("refusing to publish this staged product", strict.stderr)


if __name__ == "__main__":
    unittest.main()
