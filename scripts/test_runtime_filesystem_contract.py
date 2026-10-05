import unittest

from runtime_filesystem_contract import requires_native_filesystem


class FilesystemContractTests(unittest.TestCase):
    def test_before_filesystem_support(self):
        for version in ("0.10.4", "0.14.0", "2026.1.3", "2026.8.99"):
            with self.subTest(version=version):
                self.assertFalse(requires_native_filesystem(version))

    def test_floor_and_future_releases(self):
        for version in ("2026.9.0", "2026.9.4", "2026.10.0", "2027.1.0"):
            with self.subTest(version=version):
                self.assertTrue(requires_native_filesystem(version))

    def test_invalid_identity_does_not_select_optional_support(self):
        for version in ("", "2026.9", "2026.09.0", "v2026.9.0", "2026.9.0\n"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                requires_native_filesystem(version)


if __name__ == "__main__":
    unittest.main()
