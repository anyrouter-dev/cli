#!/usr/bin/env bash
# AnyRouter CLI installer. Downloads GitHub Release binaries (not npm).
#   curl -fsSL https://raw.githubusercontent.com/anyrouter-dev/cli/main/setup.sh | bash
#   curl -fsSL https://anyrouter.dev/setup.sh | bash
#   ANYR_CHANNEL=beta ./setup.sh --channel beta
# Assets: https://github.com/anyrouter-dev/cli/releases/download/<tag>/anyr-<os>-<arch>
set -euo pipefail

REPO="anyrouter-dev/cli"
GITHUB="https://github.com/anyrouter-dev/cli"
GITHUB_API="https://api.github.com/repos/anyrouter-dev/cli"

BIN_NAME="anyr"
BIN_DIR="${ANYR_BIN_DIR:-${HOME}/.local/bin}"
CHANNEL="${ANYR_CHANNEL:-stable}"
VERSION="${ANYR_VERSION:-}"

usage() {
  cat <<EOF
Install AnyRouter CLI (${BIN_NAME}) from GitHub Releases.

Usage:
  ./setup.sh [--channel stable|beta] [--version X.Y.Z]

Channels:
  stable  (default)  ${GITHUB}/releases/latest/download/anyr-\${os}-\${arch}
  beta               latest prerelease from ${GITHUB_API}/releases
                     then ${GITHUB}/releases/download/\${tag}/anyr-\${os}-\${arch}

Tagged version:
  ${GITHUB}/releases/download/v\${version}/anyr-\${os}-\${arch}

Env:
  ANYR_CHANNEL     stable (default) | beta
  ANYR_VERSION     install this tag (v prefix optional)
  ANYR_BIN_DIR     install directory (default: ~/.local/bin)
  ANYR_SETUP_BIN   if this is a file, copy it instead of downloading (tests)

Assets: anyr-linux-x86_64 anyr-linux-arm64 anyr-darwin-x86_64 anyr-darwin-arm64
EOF
}

die() {
  echo "setup.sh: $*" >&2
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    -h | --help)
      usage
      exit 0
      ;;
    --channel)
      [ $# -ge 2 ] || die "--channel requires a value"
      CHANNEL="$2"
      shift 2
      ;;
    --channel=*)
      CHANNEL="${1#--channel=}"
      shift
      ;;
    --version)
      [ $# -ge 2 ] || die "--version requires a value"
      VERSION="$2"
      shift 2
      ;;
    --version=*)
      VERSION="${1#--version=}"
      shift
      ;;
    --bin-dir)
      [ $# -ge 2 ] || die "--bin-dir requires a value"
      BIN_DIR="$2"
      shift 2
      ;;
    --bin-dir=*)
      BIN_DIR="${1#--bin-dir=}"
      shift
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

CHANNEL="$(printf '%s' "$CHANNEL" | tr '[:upper:]' '[:lower:]')"
case "$CHANNEL" in
  stable | beta) ;;
  *) die "unknown channel \"${CHANNEL}\" (use stable or beta)" ;;
esac

detect_os() {
  case "$(uname -s)" in
    Linux) printf 'linux' ;;
    Darwin) printf 'darwin' ;;
    *) die "unsupported OS: $(uname -s) (need linux or darwin)" ;;
  esac
}

detect_arch() {
  case "$(uname -m)" in
    x86_64 | amd64) printf 'x86_64' ;;
    arm64 | aarch64) printf 'arm64' ;;
    *) die "unsupported arch: $(uname -m) (need x86_64 or arm64)" ;;
  esac
}

install_bin() {
  local src="$1"
  [ -f "$src" ] || die "source is not a file: $src"
  mkdir -p "$BIN_DIR"
  cp "$src" "${BIN_DIR}/${BIN_NAME}"
  chmod +x "${BIN_DIR}/${BIN_NAME}"
  ln -sfn "$BIN_NAME" "${BIN_DIR}/anyrouter"
  ln -sfn "$BIN_NAME" "${BIN_DIR}/ar"
  echo "Installed ${BIN_DIR}/${BIN_NAME}"
  echo "Symlinks: ${BIN_DIR}/anyrouter ${BIN_DIR}/ar"
  case ":${PATH}:" in
    *":${BIN_DIR}:"*) ;;
    *)
      echo "Add ${BIN_DIR} to PATH:"
      echo "  export PATH=\"${BIN_DIR}:\$PATH\""
      ;;
  esac
}

# Local test mode: copy ANYR_SETUP_BIN, never hit the network.
if [ -n "${ANYR_SETUP_BIN:-}" ]; then
  [ -f "$ANYR_SETUP_BIN" ] || die "ANYR_SETUP_BIN is not a file: $ANYR_SETUP_BIN"
  echo "ANYR_SETUP_BIN=${ANYR_SETUP_BIN} (local, no download)"
  install_bin "$ANYR_SETUP_BIN"
  exit 0
fi

command -v curl >/dev/null 2>&1 || die "curl is required to download releases"

os="$(detect_os)"
arch="$(detect_arch)"
asset="anyr-${os}-${arch}"

pick_beta_tag() {
  local json
  json="$(curl -fsSL \
    -H "Accept: application/vnd.github+json" \
    -H "User-Agent: anyr-setup" \
    "${GITHUB_API}/releases")"
  if command -v python3 >/dev/null 2>&1; then
    printf '%s' "$json" | python3 -c '
import json, sys
releases = json.load(sys.stdin)
for rel in releases:
    if rel.get("draft"):
        continue
    if rel.get("prerelease"):
        tag = rel.get("tag_name") or ""
        if tag:
            print(tag)
            sys.exit(0)
sys.stderr.write("no beta (prerelease) found on GitHub Releases\n")
sys.exit(1)
'
  else
    die "python3 is required for ANYR_CHANNEL=beta"
  fi
}

if [ -n "$VERSION" ]; then
  tag="v${VERSION#v}"
  url="${GITHUB}/releases/download/${tag}/${asset}"
elif [ "$CHANNEL" = "beta" ]; then
  tag="$(pick_beta_tag)"
  [ -n "$tag" ] || die "no beta tag from GitHub API"
  url="${GITHUB}/releases/download/${tag}/${asset}"
else
  # stable: GitHub latest is the newest non-prerelease
  url="${GITHUB}/releases/latest/download/${asset}"
fi

echo "channel=${CHANNEL} os=${os} arch=${arch}"
echo "Downloading ${url}"

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
if ! curl -fsSL "$url" -o "$tmp"; then
  die "download failed: $url"
fi
if [ ! -s "$tmp" ]; then
  die "download was empty: $url"
fi
# GitHub 404 pages are HTML; release assets are ELF / Mach-O.
if head -c 16 "$tmp" | grep -q '<'; then
  die "download did not look like a binary (got HTML?): $url"
fi

install_bin "$tmp"
