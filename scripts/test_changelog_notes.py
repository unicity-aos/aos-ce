import unittest

from changelog_notes import notes


class ChangelogNotesTests(unittest.TestCase):
    def test_only_selected_release_is_emitted(self):
        text = "# Changelog\n\n## [2026.9.0] - Unreleased\n\nNew behavior\n\n## [2026.1.3] - Unreleased\n\nOld behavior\n"
        self.assertEqual(notes(text, "2026.9.0"), "New behavior\n")

    def test_missing_duplicate_and_empty_sections_fail(self):
        for text in ("", "## [v]\n", "## [v]\none\n## [v]\ntwo\n"):
            with self.subTest(text=text), self.assertRaises(ValueError):
                notes(text, "v")


if __name__ == "__main__":
    unittest.main()
