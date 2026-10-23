#!/usr/bin/env python3
"""Validate relative Markdown file and heading links in the repository."""
from pathlib import Path
import re
import sys
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r"!?(?:\[[^]]*\])\(([^)\s]+)(?:\s+[^)]*)?\)")
HEADING = re.compile(r"^#{1,6}\s+(.+?)\s*#*\s*$", re.MULTILINE)
LINE_ANCHOR = re.compile(r"^L\d+(?:[-\u2013]L?\d+)?$", re.IGNORECASE)


def slug(text: str) -> str:
    text = re.sub(r"[`*_~]", "", text).lower()
    return re.sub(r"[^a-z0-9 -]", "", text).replace(" ", "-")


def strip_fenced_code(text: str) -> str:
    lines = []
    in_fence = False
    fence_char = ""
    for line in text.splitlines(keepends=True):
        stripped = line.strip()
        if not in_fence:
            if stripped.startswith("```") or stripped.startswith("~~~"):
                in_fence = True
                fence_char = stripped[:3]
                continue
            lines.append(line)
        else:
            if stripped.startswith(fence_char):
                in_fence = False
                fence_char = ""
    return "".join(lines)


def extract_heading_anchors(content: str) -> set[str]:
    cleaned = strip_fenced_code(content)
    anchors = set()
    counts = {}
    for h in HEADING.findall(cleaned):
        s = slug(h)
        if not s:
            continue
        count = counts.get(s, 0)
        counts[s] = count + 1
        if count == 0:
            anchors.add(s)
        else:
            anchors.add(f"{s}-{count}")
    return anchors


def check_markdown_links(root: Path) -> list[str]:
    errors = []
    for markdown in root.rglob("*.md"):
        if any(part in {".git", "target", "node_modules"} for part in markdown.parts):
            continue
        content = markdown.read_text(encoding="utf-8")
        clean_content = strip_fenced_code(content)
        for raw in LINK.findall(clean_content):
            parsed = urlsplit(raw)
            if parsed.scheme or parsed.netloc:
                continue
            target = unquote(parsed.path)
            fragment = unquote(parsed.fragment)
            if not target and not fragment:
                continue
            resolved = (markdown.parent / target).resolve() if target else markdown.resolve()
            if target and not resolved.exists():
                errors.append(f"{markdown.relative_to(root)}: missing file {raw}")
                continue
            if not fragment:
                continue
            if resolved.is_dir():
                continue
            if LINE_ANCHOR.match(fragment):
                continue
            if resolved.is_file() and resolved.suffix.lower() != ".md":
                continue
            target_content = resolved.read_text(encoding="utf-8")
            anchors = extract_heading_anchors(target_content)
            if fragment.lower() not in anchors:
                errors.append(f"{markdown.relative_to(root)}: missing heading {raw}")
    return errors


def main():
    errors = check_markdown_links(ROOT)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        sys.exit(1)
    print("All relative Markdown links are valid.")


if __name__ == "__main__":
    main()
