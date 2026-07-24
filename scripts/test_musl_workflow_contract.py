#!/usr/bin/env python3
"""Regression checks for the native Linux musl workflow boundary."""

from __future__ import annotations

import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
AMD64_IMAGE = (
    "docker.io/library/rust@"
    "sha256:e98196986adced5602f6e21c54babdbf2a8700400c7a78868324a3630e0c5d15"
)
ARM64_IMAGE = (
    "docker.io/library/rust@"
    "sha256:594694ee6b07747b63b5c265be2616b62e814180b66227e2c18c6ee85e4136be"
)


def job_block(workflow: str, name: str) -> str:
    lines = workflow.splitlines()
    header = f"  {name}:"
    try:
        start = lines.index(header)
    except ValueError as error:
        raise AssertionError(f"workflow job {name!r} is missing") from error
    end = len(lines)
    for index in range(start + 1, len(lines)):
        if re.fullmatch(r"  [A-Za-z0-9_-]+:", lines[index]):
            end = index
            break
    return "\n".join(lines[start:end])


def job_needs(workflow: str, name: str) -> set[str]:
    block = job_block(workflow, name)
    match = re.search(r"^    needs: \[([^]]*)\]$", block, re.MULTILINE)
    if match is None:
        raise AssertionError(f"workflow job {name!r} has no inline needs list")
    return {dependency.strip() for dependency in match.group(1).split(",")}


class MuslWorkflowContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.ci = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        self.release = (ROOT / ".github/workflows/release.yml").read_text(
            encoding="utf-8"
        )
        self.installer_smoke = (
            ROOT / "scripts/test-clean-home-musl-install.sh"
        ).read_text(encoding="utf-8")

    def test_both_native_targets_use_exact_platform_images(self) -> None:
        for workflow in (self.ci, self.release):
            self.assertIn("target: x86_64-unknown-linux-musl", workflow)
            self.assertIn("os: ubuntu-latest", workflow)
            self.assertIn("platform: linux/amd64", workflow)
            self.assertIn(AMD64_IMAGE, workflow)
            self.assertIn("target: aarch64-unknown-linux-musl", workflow)
            self.assertIn("os: ubuntu-24.04-arm", workflow)
            self.assertIn("platform: linux/arm64", workflow)
            self.assertIn(ARM64_IMAGE, workflow)
            self.assertNotIn("setup-qemu", workflow.lower())

    def test_release_authenticates_runtime_metadata_before_packaging(self) -> None:
        legacy = self.release.index('for metadata in "$RUNTIME_LEGACY_METADATA"')
        validator = self.release.index("scripts/astrid_musl_release.py")
        package = self.release.index("scripts/package-release.sh", validator)
        self.assertLess(legacy, validator)
        self.assertLess(validator, package)
        self.assertIn("--use-signed-timestamps", self.release[legacy:validator])

    def test_release_gate_and_signed_install_smoke_are_mandatory(self) -> None:
        self.assertIn(
            "validate-release-contract.py --require-release-ready", self.release
        )
        self.assertIn("test-clean-home-musl-install.sh", self.release)
        for value in (
            '"$TARGET"',
            '"$PLATFORM"',
            '"$IMAGE"',
            '"$EXPECTED_UNAME"',
            '"$COSIGN_ASSET"',
        ):
            self.assertIn(value, self.release)
        self.assertIn("runtime-musl-compatibility.toml", self.release)
        self.assertIn("unicity-aos-*-release.toml", self.release)

    def test_signed_candidate_is_smoked_before_any_new_draft_or_publication(self) -> None:
        candidate_job = job_block(self.release, "release-candidate")
        smoke_job = job_block(self.release, "signed-musl-install")
        publish_job = job_block(self.release, "github-release")
        self.assertEqual(
            job_needs(self.release, "signed-musl-install"),
            {"classify", "release-candidate"},
        )
        self.assertEqual(
            job_needs(self.release, "github-release"),
            {"classify", "release-candidate", "signed-musl-install"},
        )
        self.assertIn('ARTIFACT_NAME="signed-release-candidate-', candidate_job)
        self.assertNotIn("Create draft release", candidate_job)
        self.assertNotIn("draft=false", candidate_job)
        self.assertIn("Create draft release with write-once assets", publish_job)
        self.assertIn("draft=false", publish_job)
        self.assertNotIn("id-token: write", smoke_job)
        self.assertNotIn("id-token: write", publish_job)
        self.assertIn("id-token: write", candidate_job)
        self.assertIn("permissions:\n      contents: read", candidate_job)
        self.assertIn("permissions:\n      contents: read", smoke_job)
        self.assertIn("permissions:\n      contents: write", publish_job)
        self.assertNotIn("attestations: write", smoke_job)
        self.assertNotIn("attestations: write", publish_job)

    def test_signed_smoke_matrix_is_native_and_target_complete(self) -> None:
        smoke = job_block(self.release, "signed-musl-install")
        for value in (
            "os: ubuntu-latest",
            "platform: linux/amd64",
            "expected_uname: x86_64",
            "cosign_asset: cosign-linux-amd64",
            "os: ubuntu-24.04-arm",
            "platform: linux/arm64",
            "expected_uname: aarch64",
            "cosign_asset: cosign-linux-arm64",
            AMD64_IMAGE,
            ARM64_IMAGE,
        ):
            self.assertIn(value, smoke)
        self.assertNotIn("setup-qemu", smoke.lower())

    def test_installer_smoke_rejects_cross_architecture_configuration(self) -> None:
        for value in (
            "x86_64-unknown-linux-musl:linux/amd64:x86_64:cosign-linux-amd64:"
            + AMD64_IMAGE,
            "aarch64-unknown-linux-musl:linux/arm64:aarch64:cosign-linux-arm64:"
            + ARM64_IMAGE,
            '[[ "$(uname -m)" == "$expected_uname" ]]',
            '--platform "$platform"',
            '--env "EXPECTED_UNAME=$expected_uname"',
        ):
            self.assertIn(value, self.installer_smoke)

    def test_static_validation_covers_aos_and_all_runtime_binaries(self) -> None:
        validation = self.release[
            self.release.index("Validate and exercise every Linux musl binary") :
        ]
        for binary in ("$AOS_BINARY", "astrid", "astrid-daemon", "astrid-build", "astrid-emit"):
            self.assertIn(binary, validation)


if __name__ == "__main__":
    unittest.main()
