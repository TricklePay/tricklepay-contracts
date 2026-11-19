#!/bin/bash

# Verifies that [profile.release] sets overflow-checks = true.
# Note: Removing overflow-checks = true or changing it to false from Cargo.toml's
# [profile.release] table causes this script to fail with exit code 1.
set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel)"
cd "$ROOT_DIR"

if awk '
    /^\[profile\.release\][[:space:]]*(#.*)?$/ {
        in_release = 1
        found_release = 1
        next
    }
    /^\[/ {
        if (in_release) {
            exit
        }
        in_release = 0
    }
    in_release && /^[[:space:]]*overflow-checks[[:space:]]*=/ {
        value = $0
        sub(/^[^=]*=/, "", value)
        sub(/#.*/, "", value)
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", value)
        if (value == "true") {
            enabled = 1
        }
    }
    END {
        if (found_release && enabled) {
            exit 0
        }
        exit 1
    }
' Cargo.toml; then
    echo "Release profile overflow checks are enabled."
else
    echo "Error: [profile.release] must set overflow-checks = true in Cargo.toml." >&2
    exit 1
fi
