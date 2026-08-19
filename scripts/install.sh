#!/usr/bin/env bash
# Wrapper around ../setup.sh.
# Channels: stable | beta. Flags: --channel --version --bin-dir.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec bash "$ROOT/setup.sh" "$@"
