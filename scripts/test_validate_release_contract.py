#!/usr/bin/env python3
"""Focused tests for the release-readiness publication gate."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("validate-release-contract.py")
SPEC = importlib.util.spec_from_file_location("validate_release_contract", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load release contract validator")
VALIDATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATOR)


def metadata(release_ready: object) -> dict[str, object]:
    return {
        "schema-version": 1,
        "runtime": {"release-ready": release_ready},
    }


class ReleaseReadinessTests(unittest.TestCase):
    def test_false_passes_staged_validation(self) -> None:
        self.assertFalse(
            VALIDATOR.validate_release_readiness(
                metadata(False), require_release_ready=False
            )
        )

    def test_false_fails_publication_validation(self) -> None:
        with self.assertRaisesRegex(ValueError, "refusing to publish"):
            VALIDATOR.validate_release_readiness(
                metadata(False), require_release_ready=True
            )

    def test_true_passes_publication_validation(self) -> None:
        self.assertTrue(
            VALIDATOR.validate_release_readiness(
                metadata(True), require_release_ready=True
            )
        )

    def test_readiness_must_be_a_boolean(self) -> None:
        with self.assertRaisesRegex(ValueError, "must be a boolean"):
            VALIDATOR.validate_release_readiness(
                metadata("false"), require_release_ready=False
            )

    def test_schema_version_is_required(self) -> None:
        value = metadata(False)
        value["schema-version"] = 2
        with self.assertRaisesRegex(ValueError, "schema-version must be 1"):
            VALIDATOR.validate_release_readiness(
                value, require_release_ready=False
            )


if __name__ == "__main__":
    unittest.main()
