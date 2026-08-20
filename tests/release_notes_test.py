#!/usr/bin/env python3
"""release-please-style notes: this version plus older versions."""
from __future__ import annotations

import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import release_notes as rn  # noqa: E402


SAMPLE = """# Changelog

This file is maintained automatically by release-please.

## [0.1.8](https://github.com/anyrouter-dev/cli/compare/v0.1.7...v0.1.8) (2026-08-20)

### Bug Fixes

* **cli:** login URL includes the code ([a5e1307](https://github.com/anyrouter-dev/cli/commit/a5e1307))

## [0.1.7](https://github.com/anyrouter-dev/cli/compare/v0.1.6...v0.1.7) (2026-08-20)

### Bug Fixes

* **cli:** keep Claude aliases distinct ([f81e971](https://github.com/anyrouter-dev/cli/commit/f81e971))
"""


class FormatTests(unittest.TestCase):
    def test_scope_and_hidden_chore(self) -> None:
        sha = "a5e130768a4f7bcaa17c7429ea4dc3738d9a39ab"
        feat = rn.format_item(sha, "fix(cli): login URL includes the code")
        self.assertIsNotNone(feat)
        kind, line = feat
        self.assertEqual(kind, "fix")
        self.assertIn("**cli:** login URL includes the code", line)
        self.assertIn(sha, line)
        self.assertIsNone(rn.format_item(sha, "chore(cli): drop GitHub Pages"))

    def test_github_notes_are_full_history_from_tag(self) -> None:
        versions = rn.parse_changelog(SAMPLE)
        notes = rn.notes_from(versions, "v0.1.8", full=True)
        self.assertTrue(notes.startswith("## [0.1.8]"))
        self.assertIn("## [0.1.7]", notes)
        only = rn.notes_from(versions, "v0.1.7", full=True)
        self.assertTrue(only.startswith("## [0.1.7]"))
        self.assertNotIn("## [0.1.8]", only)


class LiveChangelogTests(unittest.TestCase):
    def test_repo_changelog_has_compare_links(self) -> None:
        text = rn.CHANGELOG.read_text(encoding="utf-8")
        self.assertIn("googleapis/release-please", text)
        self.assertIn("## [0.1.8](https://github.com/anyrouter-dev/cli/compare/v0.1.7...v0.1.8)", text)
        notes = rn.notes_from(rn.parse_changelog(text), "v0.1.8", full=True)
        self.assertIn("## [0.1.0]", notes)
        self.assertIn("## [0.1.8]", notes)


if __name__ == "__main__":
    unittest.main()
