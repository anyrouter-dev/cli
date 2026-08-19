#!/usr/bin/env bash
# Point this repo at .githooks (prepare-commit-msg appends co-author trailers).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
git config core.hooksPath .githooks
chmod +x .githooks/prepare-commit-msg
echo "git core.hooksPath set to .githooks"
echo "Skip one commit with ANYR_SKIP_COAUTHORS=1"
