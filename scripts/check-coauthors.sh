#!/usr/bin/env bash
# Assert the commit hook sources both required co-author trailers.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOOK="${ROOT}/.githooks/prepare-commit-msg"

if [ ! -f "$HOOK" ]; then
  echo "missing hook: $HOOK" >&2
  exit 1
fi

need() {
  if ! grep -F -q "$1" "$HOOK"; then
    echo "hook is missing: $1" >&2
    exit 1
  fi
}

need "Co-authored-by: Duyet Le <me@duyet.net>"
need "Co-authored-by: duyetbot <bot@duyet.net>"
echo "co-author trailers present in $HOOK"
