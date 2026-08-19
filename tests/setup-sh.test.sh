#!/usr/bin/env bash
# Local installer tests: grep URLs/channels, then copy ANYR_SETUP_BIN (no network).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SETUP="${ROOT}/setup.sh"

[ -f "$SETUP" ] || {
  echo "missing ${SETUP}" >&2
  exit 1
}

grep -q 'github.com/anyrouter-dev/cli' "$SETUP"
grep -q 'stable' "$SETUP"
grep -q 'beta' "$SETUP"
grep -q 'releases/latest/download' "$SETUP"
grep -q 'releases/download' "$SETUP"

if grep -E 'github.com/duyet/|duyet/anyrouter' "$SETUP"; then
  echo "setup.sh must not use github.com/duyet/ download URLs" >&2
  exit 1
fi

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

fake="${tmpdir}/fake-anyr"
cat >"$fake" <<'EOF'
#!/bin/sh
echo "AnyRouter CLI"
echo "login"
echo "claude"
if [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
  echo "Usage: anyr <command>"
  echo "  login     Sign in"
  echo "  claude    Launch Claude Code"
fi
EOF
chmod +x "$fake"

bindir="${tmpdir}/bin"
echo "ANYR_SETUP_BIN=${fake} ANYR_BIN_DIR=${bindir}"
ANYR_SETUP_BIN="$fake" ANYR_BIN_DIR="$bindir" bash "$SETUP"

[ -x "${bindir}/anyr" ] || {
  echo "anyr not installed at ${bindir}/anyr" >&2
  exit 1
}
[ -L "${bindir}/anyrouter" ] || {
  echo "missing anyrouter symlink" >&2
  exit 1
}
[ -L "${bindir}/ar" ] || {
  echo "missing ar symlink" >&2
  exit 1
}

help_out="$("${bindir}/anyr" --help)"
echo "$help_out"
echo "$help_out" | grep -q 'AnyRouter CLI'
echo "$help_out" | grep -q 'login'
echo "$help_out" | grep -q 'claude'

echo "setup-sh.test.sh ok"
