# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Dropped the GitHub Pages WASM playground. Docs and install live at
  [anyrouter.dev/cli](https://anyrouter.dev/cli).

## [0.1.8] - 2026-08-20

### Changed

- `ar auth login` prints one URL with the code already in it (`?code=`),
  opens the browser when possible, and waits until you approve. No separate
  "enter this code" step.

## [0.1.7] - 2026-08-20

### Fixed

- `ar claude` session default is `anyrouter/auto`, not the bare word `auto`.
  Pinning a model no longer copies it onto opus / sonnet / haiku, so Claude
  Code's `/model` picker lists all three aliases.

### Added

- Persist Claude aliases: `claude_haiku`, `claude_sonnet`, `claude_opus`.
  Set them with `ar claude --haiku|--sonnet|--opus`, `ar models use --haiku <id>`,
  or Switch model in `ar config`.

## [0.1.6] - 2026-08-20

### Fixed

- `ar pi` writes a Pi agent dir with AnyRouter already in `models.json` and
  sets `ANYROUTER_API_KEY`. Pi does not read `PI_MODELS_JSON`, so the old wrap
  launched Pi with no provider and no key.

## [0.1.5] - 2026-08-20

### Changed

- `ar config` opens an interactive TUI (pick key, account, model, credits).
  `config path` / `config get` / `config use` stay for scripts.

## [0.1.4] - 2026-08-20

### Changed

- Auth is `ar auth login|logout|status|token|switch`, like `gh auth`.
  `login`, `logout`, and `whoami` still work as aliases.
- Root help lists CORE COMMANDS (`auth`, `config`, `keys`, `models`, `usage`)
  then LAUNCH, matching the GitHub CLI layout.

## [0.1.3] - 2026-08-20

### Changed

- Bare `ar` / `anyr` signs in when no key is stored, then opens the launcher.
- `--help` is grouped (Launch / Account / Status) instead of a flat command dump.

## [0.1.2] - 2026-08-20

### Fixed

- Release bench looks up the asset by absolute path so Linux/macOS CI can
  upload binaries (`anyr-linux-x86_64` is not on `PATH`).

## [0.1.1] - 2026-08-20

### Added

- Spawn target `pi` (Pi coding agent) alongside `claude`, `codex`, and `opencode`.

### Fixed

- Help and usage lines use the name you invoked (`ar`, `anyr`, `anyrouter`, or `npx @anyr/cli`) instead of always printing `npx @anyr/cli`.

## [0.1.0] - 2026-08-20

### Added

- Initial native CLI (`anyr`) written in Rust.

[Unreleased]: https://github.com/anyrouter-dev/cli/compare/v0.1.8...HEAD
[0.1.8]: https://github.com/anyrouter-dev/cli/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/anyrouter-dev/cli/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/anyrouter-dev/cli/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/anyrouter-dev/cli/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/anyrouter-dev/cli/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/anyrouter-dev/cli/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/anyrouter-dev/cli/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/anyrouter-dev/cli/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/anyrouter-dev/cli/releases/tag/v0.1.0
