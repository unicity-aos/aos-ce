#!/usr/bin/env python3
"""Execute the standalone installer's verified-cache storage boundaries."""

import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class VerifierCacheTest(unittest.TestCase):
    def run_cache(self, unsafe=None):
        source = (ROOT / "install.sh").read_text()
        functions = source[source.index("verifier_cache_root() {"):source.index('COSIGN_BIN="$work/cosign"')]
        with tempfile.TemporaryDirectory(prefix="aos-verifier-cache-") as raw:
            root = Path(raw)
            home = root / "aos"
            home.mkdir(mode=0o700)
            binary = root / "cosign"
            binary.write_bytes(b"checksum-verified fixture bytes\n")
            digest = hashlib.sha256(binary.read_bytes()).hexdigest()
            cache = home / "cache/verifiers" / digest
            outside = root / "outside"
            outside.mkdir()
            sentinel = outside / "sentinel"
            sentinel.write_text("preserve\n")
            if unsafe == "parent":
                (home / "cache").symlink_to(outside, target_is_directory=True)
            elif unsafe == "file":
                cache.parent.mkdir(parents=True)
                cache.symlink_to(sentinel)
            script = 'sha256_file() { shasum -a 256 "$1" | awk \'{print $1}\'; }\n'
            result = subprocess.run(
                ["sh", "-c", "set -eu\n" + script + functions + "\ncache_verified_verifier\n"],
                env={**os.environ, "AOS_HOME": str(home), "COSIGN_BIN": str(binary),
                     "cosign_sha256": digest},
                capture_output=True, text=True, timeout=5, check=False,
            )
            if unsafe is None:
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(cache.read_bytes(), binary.read_bytes())
                self.assertEqual(cache.stat().st_mode & 0o777, 0o700)
                self.assertEqual(cache.parent.stat().st_mode & 0o777, 0o700)
                self.assertFalse(list(cache.parent.glob(".cosign.*")))
            else:
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(sentinel.read_text(), "preserve\n")
                self.assertEqual(list(outside.iterdir()), [sentinel])

    def test_verified_bytes_are_cached_privately(self):
        self.run_cache()

    def test_symlinked_parent_is_refused(self):
        self.run_cache("parent")

    def test_symlinked_entry_is_refused(self):
        self.run_cache("file")


if __name__ == "__main__":
    unittest.main()
