#!/usr/bin/env bash
set -eu

# release-tag.sh: Create a release tag with verified build, hash, and changelog.
#
# This script:
# 1. Verifies the working tree is clean
# 2. Runs the release-check target (manifest, build, hash, changelog)
# 3. Extracts the version and changelog section
# 4. Computes the WASM hash
# 5. Drafts release notes with the hash and commit
# 6. Asks for confirmation before creating the tag

# Check if working tree is clean
if [ -n "$(git status --short)" ]; then
    echo "Error: working tree is dirty. Commit or stash all changes before releasing." >&2
    exit 1
fi

echo "Running release checks..."
make release-check > /dev/null 2>&1 || {
    echo "Error: release-check failed" >&2
    exit 1
}

# Extract version from CHANGELOG
version=$(grep -m 1 '^## \[' CHANGELOG.md | sed 's/## \[//;s/\].*//')
echo "Release version: $version"

# Compute WASM hash
cargo build --locked --release --target wasm32v1-none > /dev/null 2>&1
if command -v sha256sum >/dev/null 2>&1; then
    hash=$(sha256sum target/wasm32v1-none/release/tricklepay_stream.wasm | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
    hash=$(shasum -a 256 target/wasm32v1-none/release/tricklepay_stream.wasm | awk '{print $1}')
else
    echo "Error: install sha256sum or shasum to compute the WASM SHA-256 hash." >&2
    exit 1
fi

echo "WASM hash: sha256:$hash"

# Extract the changelog section for this version
# This finds the section from "## [version]" to the next "## [" or end of file
start_line=$(grep -n "^## \[$version\]" CHANGELOG.md | head -1 | cut -d: -f1)
if [ -z "$start_line" ]; then
    echo "Error: version $version not found in CHANGELOG.md" >&2
    exit 1
fi

# Find the next section or end of file
next_line=$(tail -n +$((start_line + 1)) CHANGELOG.md | grep -n "^## \[" | head -1 | cut -d: -f1)
if [ -n "$next_line" ]; then
    end_line=$((start_line + next_line - 2))
else
    end_line=$(wc -l < CHANGELOG.md)
fi

changelog_section=$(sed -n "${start_line},${end_line}p" CHANGELOG.md)

# Get commit hash
commit=$(git rev-parse --short HEAD)

# Draft the release notes
tag_name="v$version"
release_notes="Release $version

This release was verified with:
- Locked dependency build
- Manifest check
- WASM hash: sha256:$hash
- Commit: $(git rev-parse HEAD)

## Changes

$changelog_section
"

echo ""
echo "=== Draft Release Notes ==="
echo "$release_notes"
echo "=== End Release Notes ==="
echo ""
echo "Tag name: $tag_name"
echo ""

# Ask for confirmation
read -p "Create annotated tag $tag_name? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Aborted."
    exit 0
fi

# Create the annotated tag
git tag -a "$tag_name" -m "$release_notes"
echo "✓ Tag $tag_name created successfully"
echo ""
echo "Next: git push origin $tag_name"
