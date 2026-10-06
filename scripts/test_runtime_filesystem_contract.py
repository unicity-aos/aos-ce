import unittest
from pathlib import Path
import subprocess

from runtime_filesystem_contract import requires_native_filesystem


class FilesystemContractTests(unittest.TestCase):
    def test_public_installer_membership_agrees_with_composer(self):
        installer = (Path(__file__).resolve().parent.parent / "install.sh").read_text()
        selection = "# The signed runtime tuple" + installer.split(
            "# The signed runtime tuple", 1)[1].split('target_section=', 1)[0]
        for version in ("0.10.4", "0.14.0", "2026.8.99", "2026.9.0",
                        "2026.9.4", "2026.10.0", "2026.10.0-rc.1", "2027.1.0", "99999999999999999999.0.0"):
            for os_name in ("Linux", "Darwin"):
                with self.subTest(version=version, os=os_name):
                    result = subprocess.run(
                        ["sh", "-c", 'os=$1; runtime_version=$2; runtime_binaries=astrid;\n'
                         + selection + '\nprintf "%s" "$runtime_binaries"',
                         "selection", os_name, version], text=True, capture_output=True, check=True)
                    self.assertEqual("astrid-storage-provider-fuse" in result.stdout,
                                     os_name == "Linux" and requires_native_filesystem(version))
                    self.assertEqual(result.stderr, "")

    def test_before_filesystem_support(self):
        for version in ("0.10.4", "0.14.0", "2026.1.3", "2026.8.99"):
            with self.subTest(version=version):
                self.assertFalse(requires_native_filesystem(version))

    def test_floor_and_future_releases(self):
        for version in ("2026.9.0", "2026.9.4", "2026.10.0", "2026.10.0-rc.1", "2027.1.0"):
            with self.subTest(version=version):
                self.assertTrue(requires_native_filesystem(version))

    def test_invalid_identity_does_not_select_optional_support(self):
        for version in ("", "2026.9", "2026.09.0", "v2026.9.0", "2026.9.0\n", "2026.10.0-rc.0", "2026.10.0-rc.01"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                requires_native_filesystem(version)


if __name__ == "__main__":
    unittest.main()
