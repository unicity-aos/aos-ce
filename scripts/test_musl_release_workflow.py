#!/usr/bin/env python3
"""Keep native musl production connected to signing and publication."""

from pathlib import Path
import re
import subprocess
import unittest


ROOT = Path(__file__).resolve().parent.parent


class MuslReleaseWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.workflow = (ROOT / ".github/workflows/release.yml").read_text()

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
