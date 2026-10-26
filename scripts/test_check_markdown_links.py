#!/usr/bin/env python3
"""Tests for scripts/check-markdown-links.py using only the standard library."""
from pathlib import Path
import sys
import tempfile
import unittest

# Ensure the scripts directory is in sys.path
SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

# Import the check function directly
import importlib.util
spec = importlib.util.spec_from_file_location(
    "check_markdown_links", SCRIPTS_DIR / "check-markdown-links.py"
)
check_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_module)
check_markdown_links = check_module.check_markdown_links


class TestCheckMarkdownLinks(unittest.TestCase):
    def test_missing_file(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            md = root / "index.md"
            md.write_text("[missing link](nonexistent.md)", encoding="utf-8")
            errors = check_markdown_links(root)
            self.assertEqual(len(errors), 1)
            self.assertIn("missing file nonexistent.md", errors[0])

    def test_missing_heading(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            target = root / "doc.md"
            target.write_text("# Existing Heading\nContent", encoding="utf-8")
            md = root / "index.md"
            md.write_text("[bad anchor](doc.md#nonexistent-heading)", encoding="utf-8")
            errors = check_markdown_links(root)
            self.assertEqual(len(errors), 1)
            self.assertIn("missing heading doc.md#nonexistent-heading", errors[0])

    def test_line_anchor(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            code_file = root / "lib.rs"
            code_file.write_text("pub fn sample() {}\n" * 30, encoding="utf-8")
            md = root / "index.md"
            md.write_text(
                "[single line](lib.rs#L10)\n[range](lib.rs#L10-L20)",
                encoding="utf-8",
            )
            errors = check_markdown_links(root)
            self.assertEqual(errors, [])

    def test_en_dash_line_anchor(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            code_file = root / "lib.rs"
            code_file.write_text("pub fn sample() {}\n" * 30, encoding="utf-8")
            md = root / "index.md"
            md.write_text(
                "[range](lib.rs#L457\u2013L472)",
                encoding="utf-8",
            )
            errors = check_markdown_links(root)
            self.assertEqual(errors, [])

    def test_missing_file_with_line_anchor(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            md = root / "index.md"
            md.write_text("[missing](missing.rs#L10)", encoding="utf-8")
            errors = check_markdown_links(root)
            self.assertEqual(len(errors), 1)
            self.assertIn("missing file missing.rs#L10", errors[0])

    def test_directory_link(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            sub = root / "contracts"
            sub.mkdir()
            md = root / "index.md"
            md.write_text(
                "[contracts directory](contracts/)\n[without slash](contracts)",
                encoding="utf-8",
            )
            errors = check_markdown_links(root)
            self.assertEqual(errors, [])

    def test_duplicate_headings(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            doc = root / "doc.md"
            doc.write_text(
                "# Title\n\n## Section\nFirst\n\n## Section\nSecond\n\n## Section\nThird\n",
                encoding="utf-8",
            )
            md = root / "index.md"
            md.write_text(
                "[first](doc.md#section)\n[second](doc.md#section-1)\n[third](doc.md#section-2)",
                encoding="utf-8",
            )
            errors = check_markdown_links(root)
            self.assertEqual(errors, [])

    def test_fenced_code_block(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            md = root / "index.md"
            md.write_text(
                "# Heading\n\n"
                "```markdown\n"
                "[fake link in code fence](missing.md)\n"
                "[fake heading anchor](#missing-heading)\n"
                "# Fake Heading In Fence\n"
                "```\n\n"
                "Valid text outside fences with [real link](#heading).\n",
                encoding="utf-8",
            )
            errors = check_markdown_links(root)
            self.assertEqual(errors, [])


if __name__ == "__main__":
    unittest.main()
