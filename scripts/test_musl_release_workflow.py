#!/usr/bin/env python3
"""Keep native musl production connected to signing and publication."""

from pathlib import Path
import os
import re
import subprocess
import tempfile
import tarfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parent.parent


class MuslReleaseWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.workflow = (ROOT / ".github/workflows/release.yml").read_text()

    def test_signed_archive_listing_drains_tar_and_rejects_missing_signature(self):
        block = self.workflow.split("          signed_archives=0\n", 1)[1].split(
            "      - name: Generate checksums", 1)[0]
        script = 'set -euo pipefail\nassets=("$1")\nsigned_archives=0\n' + textwrap.dedent(block)
        with tempfile.TemporaryDirectory() as temp:
            for signed in (True, False):
                archive = Path(temp) / f"bundle-{signed}.tar.gz"
                with tarfile.open(archive, "w:gz") as tar:
                    if signed:
                        tar.addfile(tarfile.TarInfo("product/Distro.sig"))
                    # Larger than a pipe buffer, with the signature first.
                    for index in range(10000):
                        tar.addfile(tarfile.TarInfo(f"product/capsules/member-{index:05d}"))
                result = subprocess.run(["bash", "-c", script, "check", str(archive)],
                                        text=True, capture_output=True)
                self.assertEqual(result.returncode, 0 if signed else 1, result.stderr)
                self.assertNotIn("write error", result.stderr)

    def test_gnu_clean_home_uses_container_built_probe(self):
        build = self.workflow.split("- name: Build Linux product binary", 1)[1].split(
            "- name: Build native static musl", 1)[0]
        self.assertIn('-p unicity-aos-bootstrap --example init_provenance', build)
        self.assertIn('if [[ "$TARGET" == x86_64-unknown-linux-gnu ]]', build)
        test = self.workflow.split("- name: Test a clean Community Edition home", 1)[1]
        self.assertIn('"$PWD/target/glibc-2.31/${{ matrix.target }}/release/examples/init_provenance"', test)

    def test_prebuilt_probe_does_not_invoke_host_cargo(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            probe = root / "probe"
            probe.write_text("#!/bin/sh\nexit 0\n")
            probe.chmod(0o700)
            cargo = root / "cargo"
            marker = root / "cargo-invoked"
            cargo.write_text(f'#!/bin/sh\ntouch "{marker}"\nexit 99\n')
            cargo.chmod(0o700)
            env = dict(os.environ, PATH=f"{root}:{os.environ['PATH']}")
            result = subprocess.run(
                ["bash", str(ROOT / "scripts/test-clean-home-init.sh"), str(root), str(probe)],
                env=env, text=True, capture_output=True)
            # Reach bundle validation without trying to write a host Cargo tree.
            self.assertEqual(result.returncode, 1)
            self.assertIn("clean-home init bundle is missing bin/aos", result.stderr)
            self.assertFalse(marker.exists())

    def test_native_architecture_matrix(self):
        build = self.workflow.split("\n  build:", 1)[1].split("\n  capsules:", 1)[0]
        for architecture, runner in (
            ("x86_64", "ubuntu-24.04"),
            ("aarch64", "ubuntu-24.04-arm"),
        ):
            self.assertIn(
                f"- target: {architecture}-unknown-linux-musl\n            os: {runner}\n",
                build,
            )
        self.assertIn('test "$(uname -m)" = "${TARGET%%-*}"', build)
        self.assertIn('python3 scripts/check_static_elf.py', build)
        self.assertIn('astrid-storage-provider-fuse', build)
        self.assertLess(build.index("Verify runtime release identity"),
                        build.index("Verify static musl runtime executables"))

    def test_metadata_and_publication_require_both_archives(self):
        text = self.workflow
        self.assertIn("python3 scripts/musl_release_metadata.py render", text)
        self.assertIn("--runtime-pin release/runtime-musl-compatibility.toml", text)
        self.assertIn('artifacts/unicity-aos-${GITHUB_REF_NAME}-musl-release.toml', text)
        calls = text.split("python3 scripts/release_publication.py ")[1:]
        self.assertEqual(len(calls), 2)
        for call in calls:
            self.assertIn("--require-musl", call.split("--artifacts", 1)[0])
        # Existing globs include the extension in both signing and upload.
        self.assertIn("runtime-compatibility.toml unicity-aos-*-release.toml; do", text)
        self.assertIn("artifacts/unicity-aos-*-release.toml", text)

    def test_embedded_run_blocks_have_valid_shell_syntax(self):
        lines = self.workflow.splitlines()
        for index, line in enumerate(lines):
            if line == "        run: |":
                block = []
                for following in lines[index + 1:]:
                    if following and not following.startswith("          "):
                        break
                    block.append(following[10:])
                script = re.sub(r"\$\{\{.*?\}\}", "workflow_value", "\n".join(block))
                result = subprocess.run(["bash", "-n"], input=script, text=True,
                                        capture_output=True)
                self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
