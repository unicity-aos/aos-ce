#!/usr/bin/env python3
"""Run numbered RC staging and the actual tag-push classifier."""

import io
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest

import release_candidate
from test_nightly_version import NightlyVersionTests


class ReleaseCandidateTests(NightlyVersionTests):
    def test_full_package_contract_after_candidate_staging(self) -> None:
        source = release_candidate.nightly_version.ROOT
        archive = subprocess.run(["git", "archive", "HEAD"], cwd=source,
                                 check=True, capture_output=True).stdout
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            with tarfile.open(fileobj=io.BytesIO(archive)) as files:
                files.extractall(root, filter="data")
            # Exercise the working script, including an uncommitted regression fix.
            shutil.copy2(source / "scripts/test-package-release.sh",
                         root / "scripts/test-package-release.sh")
            base = release_candidate.nightly_version.canonical_base(root)
            release_candidate.stage(root, f"{base}-rc.1")
            run = subprocess.run(["bash", "scripts/test-package-release.sh"],
                                 cwd=root, capture_output=True, text=True, timeout=120)
            self.assertEqual(run.returncode, 0, run.stdout + run.stderr)

    def test_shipped_posix_installer_version_classes(self) -> None:
        text = (release_candidate.nightly_version.ROOT / "install.sh").read_text()
        functions = "is_aos_nightly_version()" + text.split("is_aos_nightly_version()", 1)[1].split('if [ -n "$AOS_VERSION"', 1)[0]
        for channel, version, success in (("dev", "2026.10.0-rc.1", True), ("stable", "2026.10.0-rc.1", False), ("nightly", "2026.10.0-rc.1", False), ("dev", "2026.10.0-rc.0", False), ("dev", "2026.10.0-rc.01", False), ("stable", "2026.10.0", True)):
            with self.subTest(channel=channel, version=version):
                run = subprocess.run(["sh", "-c", functions + '\nis_aos_channel_version "$1" "$2"', "sh", channel, version], capture_output=True)
                self.assertEqual(run.returncode == 0, success, run.stderr)
    def test_stage_keeps_runtime_provenance_and_release_date(self) -> None:
        version = "2026.9.0-rc.1"
        release_candidate.stage(self.root, version)
        for name in ("crates/unicity-aos-bootstrap/Cargo.toml", "Cargo.lock", "release/runtime-compatibility.toml", "distros/community/unicity-ce/Distro.toml"):
            self.assertIn(version, (self.root / name).read_text())
        self.assertIn('version = "0.9.4"', (self.root / "release/runtime-compatibility.toml").read_text())
        self.assertIn('release-date = "2026-07-10"', (self.root / "distros/community/unicity-ce/Distro.toml").read_text())

    def test_invalid_candidate_does_not_stage(self) -> None:
        before = (self.root / "Cargo.lock").read_bytes()
        for version in ("2026.10.0-rc.1", "2026.9.0-rc.0", "2026.9.0-rc.01", "2026.9.0-beta.1", "2026.9.0-rc.1+build"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                release_candidate.stage(self.root, version)
            self.assertEqual(before, (self.root / "Cargo.lock").read_bytes())

    def test_actual_push_classifier(self) -> None:
        root = release_candidate.nightly_version.ROOT
        workflow = (root / ".github/workflows/release.yml").read_text()
        body = workflow.split("      - name: Bind event, tag, and source commit\n", 1)[1].split("\n  validate-release:", 1)[0].split("        run: |\n", 1)[1]
        body = "\n".join(line[10:] for line in body.splitlines())
        base = release_candidate.nightly_version.canonical_base(root)
        for version, success, prerelease in ((base, True, "false"), (f"{base}-rc.1", True, "true"), (f"{base}-rc.0", False, None), (f"{base}-beta.1", False, None), ("2026.0.0-rc.1", False, None)):
            with self.subTest(version=version), tempfile.NamedTemporaryFile() as output:
                env = dict(os.environ, EVENT_NAME="push", GITHUB_REF=f"refs/tags/{version}", GITHUB_REF_NAME=version, GITHUB_OUTPUT=output.name)
                run = subprocess.run(["bash", "-euo", "pipefail", "-c", body], cwd=root, env=env, capture_output=True, text=True)
                self.assertEqual(run.returncode == 0, success, run.stderr)
                if success:
                    self.assertIn(f"prerelease={prerelease}\n", output.read().decode())


if __name__ == "__main__":
    unittest.main()
