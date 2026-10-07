#!/usr/bin/env python3
"""Exercise the installer timestamp parser with native and BusyBox date."""

from pathlib import Path
import os
import re
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class TimestampTest(unittest.TestCase):
    def test_timestamp_parsing(self) -> None:
        source = (ROOT / "install.sh").read_text()
        match = re.search(r"^utc_epoch\(\) \{\n.*?^\}", source, re.M | re.S)
        self.assertIsNotNone(match)
        native_date = shutil.which("date")
        busybox = shutil.which("busybox")
        if os.environ.get("AOS_TEST_REQUIRE_BUSYBOX") == "1":
            self.assertIsNotNone(busybox, "BusyBox coverage is required in this job")
        engines = [None] + ([busybox] if busybox else [])
        with tempfile.TemporaryDirectory(prefix="aos-date-test-") as raw:
            root = Path(raw)
            for engine in engines:
                if engine:
                    shim = root / "date"
                    shim.write_text(f'#!/bin/sh\nexec "{engine}" date "$@"\n')
                    shim.chmod(0o700)
                elif (root / "date").exists():
                    (root / "date").unlink()
                env = {**os.environ, "PATH": f"{root}:{Path(native_date).parent}:/usr/bin:/bin"}
                for value, expected in (
                    ("2026-10-07T00:41:35Z", "1791333695"),
                    ("2024-02-29T12:00:00Z", "1709208000"),
                    ("2026-02-30T00:00:00Z", None),
                    ("2026-13-01T00:00:00Z", None),
                    ("2026-10-07T24:00:00Z", None),
                    ("2026-10-07T00:41:35Zjunk", None),
                    ("2026-10-07T00:41:35+00:00", None),
                    ("not-a-time", None),
                ):
                    with self.subTest(engine=engine or "native", value=value):
                        result = subprocess.run(
                            ["sh", "-c", match.group(0) + '\nutc_epoch "$1"', "test", value],
                            env=env, capture_output=True, text=True, timeout=5,
                        )
                        if expected is None:
                            self.assertNotEqual(result.returncode, 0, result.stdout)
                        else:
                            self.assertEqual(result.returncode, 0, result.stderr)
                            self.assertEqual(result.stdout.strip(), expected)


if __name__ == "__main__":
    unittest.main()
