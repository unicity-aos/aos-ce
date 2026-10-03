"""Release qualification must reject poisoned-instance runtime sources."""
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import runtime_recovery


class RecoveryQualification(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.source = Path(self.temp.name)
        (self.source / "Cargo.toml").write_text(
            '[workspace.package]\nversion = "2026.9.5"\n')
        self.commit = "a" * 40

    def git(self, source, *args):
        if args == ("rev-parse", "HEAD"):
            return self.commit
        if args[0] == "status":
            return ""
        if args[0] == "merge-base":
            return ""
        self.fail(f"unexpected git command {args}")

    def test_exact_fixed_source_accepted(self):
        with patch.object(runtime_recovery, "git", side_effect=self.git) as git:
            runtime_recovery.validate_source(self.source, self.commit, "2026.9.5")
        self.assertIn(unittest.mock.call(self.source, "merge-base", "--is-ancestor",
                                        runtime_recovery.RECOVERY_FIX, self.commit),
                      git.call_args_list)

    def test_old_source_rejected(self):
        def old(source, *args):
            if args[0] == "merge-base":
                raise subprocess.CalledProcessError(1, "git")
            return self.git(source, *args)
        with patch.object(runtime_recovery, "git", side_effect=old):
            with self.assertRaisesRegex(ValueError, "#2010"):
                runtime_recovery.validate_source(self.source, self.commit, "2026.9.5")

    def test_wrong_checkout_rejected(self):
        with patch.object(runtime_recovery, "git", return_value="b" * 40):
            with self.assertRaisesRegex(ValueError, "source commit"):
                runtime_recovery.validate_source(self.source, self.commit, "2026.9.5")

    def test_dirty_checkout_rejected(self):
        def dirty(source, *args):
            return " M source.rs" if args[0] == "status" else self.git(source, *args)
        with patch.object(runtime_recovery, "git", side_effect=dirty):
            with self.assertRaisesRegex(ValueError, "clean"):
                runtime_recovery.validate_source(self.source, self.commit, "2026.9.5")

    def test_version_mismatch_rejected(self):
        with patch.object(runtime_recovery, "git", side_effect=self.git):
            with self.assertRaisesRegex(ValueError, "version"):
                runtime_recovery.validate_source(self.source, self.commit, "2026.9.4")

    def test_malformed_commit_rejected_before_git(self):
        with patch.object(runtime_recovery, "git") as git:
            with self.assertRaises(ValueError):
                runtime_recovery.validate_source(self.source, "main", "2026.9.5")
            git.assert_not_called()

    def test_missing_or_ignored_regressions_rejected(self):
        valid = "\n".join(f"{runtime_recovery.TEST_MODULE}::{name}: test"
                          for name in runtime_recovery.REQUIRED_TESTS)
        runtime_recovery.validate_test_list(valid)
        with self.assertRaisesRegex(ValueError, "missing"):
            runtime_recovery.validate_test_list("0 tests, 0 benchmarks")
        runtime_recovery.validate_test_result("test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out")
        for output in ("test result: ok. 0 passed; 0 failed; 0 ignored",
                       "test result: ok. 8 passed; 0 failed; 1 ignored",
                       "test result: FAILED. 7 passed; 1 failed; 0 ignored"):
            with self.subTest(output=output), self.assertRaises(ValueError):
                runtime_recovery.validate_test_result(output)


if __name__ == "__main__":
    unittest.main()
