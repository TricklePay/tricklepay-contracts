#!/usr/bin/env python3
"""Tests for scripts/check-release-overflow.sh."""
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPTS_DIR.parent
CHECK_SCRIPT = SCRIPTS_DIR / "check-release-overflow.sh"


class TestCheckReleaseOverflow(unittest.TestCase):
    def test_current_repo_passes(self):
        """Current repository configuration must pass the check."""
        result = subprocess.run(
            ["bash", "scripts/check-release-overflow.sh"],
            cwd=str(REPO_ROOT),
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("Release profile overflow checks are enabled.", result.stdout)

    def _setup_repo(self, tmpdir: str, cargo_toml_content: str):
        tmppath = Path(tmpdir)
        scripts_dir = tmppath / "scripts"
        scripts_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy(CHECK_SCRIPT, scripts_dir / "check-release-overflow.sh")
        cargo_toml = tmppath / "Cargo.toml"
        cargo_toml.write_text(cargo_toml_content, encoding="utf-8")
        subprocess.run(["git", "init"], cwd=str(tmppath), capture_output=True)
        return tmppath

    def test_missing_overflow_checks_fails(self):
        """Removing overflow-checks setting from [profile.release] must fail."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmppath = self._setup_repo(
                tmpdir,
                """[workspace]
members = ["contracts/*"]

[profile.release]
opt-level = "z"
""",
            )
            result = subprocess.run(
                ["bash", "scripts/check-release-overflow.sh"],
                cwd=str(tmppath),
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(
                "Error: [profile.release] must set overflow-checks = true",
                result.stderr,
            )

    def test_overflow_checks_false_fails(self):
        """Setting overflow-checks = false must fail."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmppath = self._setup_repo(
                tmpdir,
                """[workspace]
members = ["contracts/*"]

[profile.release]
opt-level = "z"
overflow-checks = false
""",
            )
            result = subprocess.run(
                ["bash", "scripts/check-release-overflow.sh"],
                cwd=str(tmppath),
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(
                "Error: [profile.release] must set overflow-checks = true",
                result.stderr,
            )

    def test_missing_profile_release_fails(self):
        """Omitting the [profile.release] table entirely must fail."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmppath = self._setup_repo(
                tmpdir,
                """[workspace]
members = ["contracts/*"]
""",
            )
            result = subprocess.run(
                ["bash", "scripts/check-release-overflow.sh"],
                cwd=str(tmppath),
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
