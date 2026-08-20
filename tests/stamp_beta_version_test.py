#!/usr/bin/env python3
from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import stamp_beta_version as stamp  # noqa: E402


class NextBetaTests(unittest.TestCase):
    def test_bumps_patch_and_appends_run(self) -> None:
        self.assertEqual(stamp.next_beta("0.1.8", 42), "0.1.9-beta.42")
        self.assertEqual(stamp.next_beta("v0.1.8", 1), "0.1.9-beta.1")

    def test_strips_existing_prerelease_base(self) -> None:
        self.assertEqual(stamp.next_beta("0.1.8-beta.9", 10), "0.1.9-beta.10")

    def test_rejects_run_zero(self) -> None:
        with self.assertRaises(ValueError):
            stamp.next_beta("0.1.8", 0)


class StampTextTests(unittest.TestCase):
    def test_keeps_release_please_marker(self) -> None:
        src = '[package]\nversion = "0.1.8" # x-release-please-version\n'
        out = stamp.stamp_text(src, "0.1.9-beta.7")
        self.assertIn('version = "0.1.9-beta.7" # x-release-please-version', out)
        self.assertEqual(stamp.cargo_version(out), "0.1.9-beta.7")

    def test_print_only_does_not_write(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cargo = Path(tmp) / "Cargo.toml"
            cargo.write_text('[package]\nversion = "0.1.8"\n', encoding="utf-8")
            code = stamp.main(["--run", "3", "--print", "--cargo", str(cargo)])
            self.assertEqual(code, 0)
            self.assertEqual(stamp.cargo_version(cargo.read_text(encoding="utf-8")), "0.1.8")


if __name__ == "__main__":
    unittest.main()
