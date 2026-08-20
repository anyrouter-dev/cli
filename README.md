# AnyRouter CLI (`anyr`)

Native CLI for [AnyRouter](https://anyrouter.dev). This line is **0.1.x**.

[![CI](https://github.com/anyrouter-dev/cli/actions/workflows/ci.yml/badge.svg)](https://github.com/anyrouter-dev/cli/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/anyrouter-dev/cli/branch/main/graph/badge.svg)](https://codecov.io/gh/anyrouter-dev/cli)

Launch coding agents through the AnyRouter gateway. One key, every provider.

## Install

### curl (GitHub Releases)

The installer downloads `anyr-<os>-<arch>` from
[GitHub Releases](https://github.com/anyrouter-dev/cli/releases)
into `~/.local/bin`.

**stable** (latest non-prerelease):

```bash
curl -fsSL https://raw.githubusercontent.com/anyrouter-dev/cli/main/setup.sh | bash
```

**beta** (latest GitHub prerelease):

```bash
curl -fsSL https://raw.githubusercontent.com/anyrouter-dev/cli/main/setup.sh | bash -s -- --channel beta
```

Docs and install: [anyrouter.dev/cli](https://anyrouter.dev/cli).

CI builds **linux x86_64/arm64**, **macOS x86_64/arm64**, **Windows x86_64**, and **wasm32**.
Every PR runs the full test suite (`cargo test --all-targets`) on each of those
platforms, plus LLVM coverage uploaded to [Codecov](https://codecov.io/gh/anyrouter-dev/cli).
Every PR and GitHub Release gets a size + startup report (`anyr --version` / `--help`).

Manual (Linux x86_64 example):

```bash
curl -fsSL -o anyr \
  https://github.com/anyrouter-dev/cli/releases/latest/download/anyr-linux-x86_64
chmod +x anyr
```

Hosted setup (same binary):

```bash
curl -fsSL https://anyrouter.dev/setup.sh | bash
```

Add `~/.local/bin` to `PATH` if the installer says so.

### npm / npx

npm already has `@anyr/cli@0.2.8` on the **`latest`** tag (the previous JS CLI)
and `@anyr/cli@0.1.0` was published earlier, so **this repo cannot republish
0.1.0**. Native line stays **0.1.x**; the first npm publish from this repo will
be `0.1.1` (or later) with `--tag next` so `@latest` stays `0.2.8` until a
human retags.

```bash
npx @anyr/cli@next --help
npx @anyr/cli@next login
npm install -g @anyr/cli@next
```

The npm package is a thin Node wrapper. It execs a shipped binary in
`binaries/` when present, otherwise it downloads the matching GitHub Release
asset (`anyr-linux-x86_64`, `anyr-linux-arm64`, `anyr-darwin-x86_64`, `anyr-darwin-arm64`, `anyr-windows-x86_64.exe`).

## Upgrade

Re-run the installer for your channel, or bump the npm `next` tag:

```bash
# curl, stable
curl -fsSL https://raw.githubusercontent.com/anyrouter-dev/cli/main/setup.sh | bash

# curl, beta
curl -fsSL https://raw.githubusercontent.com/anyrouter-dev/cli/main/setup.sh | bash -s -- --channel beta

anyr upgrade
anyr upgrade --check --channel beta

# npm
npm install -g @anyr/cli@next
```

## Quick start

```bash
anyr --help
anyr login                  # browser / device code / paste; key saved on this machine
anyr login --device         # headless / SSH
anyr usage                  # credit balance
anyr models use <id>        # persist default model
anyr keys list              # switch with: anyr keys use
anyr account use work       # switch saved profiles
anyr claude --ok            # launch through the gateway
anyr claude --install --ok  # install Claude Code first if missing
anyr codex --ok
anyr opencode --ok
anyr pi --ok
```

## Version lock (0.1.x)

`Cargo.toml`, `package.json`, and `.release-please-manifest.json` stay on
`0.1.x`. release-please uses `"versioning": "always-bump-patch"` so `feat`,
`feat!`, and `BREAKING CHANGE` all stay `0.1.x` (no 0.2.0 / 1.0.0 path).

Do not auto-merge release-please PRs.

## Git hooks

`.githooks/prepare-commit-msg` appends:

```
Co-authored-by: Duyet Le <me@duyet.net>
Co-authored-by: duyetbot <bot@duyet.net>
```

Enable:

```bash
./scripts/install-hooks.sh
# same as: git config core.hooksPath .githooks
```

Skip one commit: `ANYR_SKIP_COAUTHORS=1 git commit`

## License

MIT — Duyet Le
