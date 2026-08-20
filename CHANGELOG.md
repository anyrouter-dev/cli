# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/anyrouter-dev/cli/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/anyrouter-dev/cli/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/anyrouter-dev/cli/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/anyrouter-dev/cli/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/anyrouter-dev/cli/releases/tag/v0.1.0
