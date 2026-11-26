#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
hooks_dir="$repo_root/scripts/hooks"

chmod +x "$hooks_dir/pre-commit"
git -C "$repo_root" config core.hooksPath scripts/hooks

echo "Installed optional Git hooks from scripts/hooks."
echo "Skip for a single commit with: SKIP_PRE_COMMIT_HOOKS=1 git commit ..."
echo "Remove with: git config --unset core.hooksPath"
