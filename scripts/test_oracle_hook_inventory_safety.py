"""Test the real QA harness guard without starting or initializing a runtime."""
from pathlib import Path
import tempfile
import unittest

from test_oracle_hook_inventory_runtime import resolve_qa_paths


class HarnessSafety(unittest.TestCase):
    def test_marker_must_be_the_parsed_distro_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for spoof in (
                '# id = "oracle-bus-qa"\n[distro]\nid = "production"\n',
                '[distro]\nid = "production"\n[note]\nid = "oracle-bus-qa"\n',
            ):
                (root / "Distro.toml").write_text(spoof)
                with self.assertRaisesRegex(RuntimeError, "unmarked"):
                    resolve_qa_paths(root, root, root, root)

    def test_malformed_distro_is_not_an_initialization_permission(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Distro.toml").write_text('id = "oracle-bus-qa"\n[distro')
            with self.assertRaises(ValueError):
                resolve_qa_paths(root, root, root, root)

    def test_relative_binaries_are_bound_before_runtime_cwd_changes(self):
        with tempfile.TemporaryDirectory(dir=Path.cwd()) as directory:
            root = Path(directory)
            (root / "Distro.toml").write_text('[distro]\nid = "oracle-bus-qa"\n')
            for name in ("astrid", "aos"):
                (root / name).touch()
            relative = root.relative_to(Path.cwd())
            selected = resolve_qa_paths(root, relative / "astrid", relative / "aos", root)
            self.assertEqual(selected, tuple(path.resolve() for path in
                                           (root, root / "astrid", root / "aos", root)))


if __name__ == "__main__":
    unittest.main()
