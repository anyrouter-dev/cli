# Changelog

This file is maintained automatically by [release-please](https://github.com/googleapis/release-please).

GitHub Releases use this file as the release notes (full history through that tag).

## [0.1.10](https://github.com/anyrouter-dev/cli/compare/v0.1.9...v0.1.10) (2026-08-21)


### Features

* **ci:** build every platform, bench size/startup, ship wasm demo ([532a1e0](https://github.com/anyrouter-dev/cli/commit/532a1e08f31a8bcd2567cc5db2269a30ff9f27e4))
* **cli:** auto-update in the background by default ([f3d5edd](https://github.com/anyrouter-dev/cli/commit/f3d5edd43c3185153af4ac88647670ea7f1faff2))
* **cli:** device login, key/model switch, agent install, color TUI ([cc4cccd](https://github.com/anyrouter-dev/cli/commit/cc4cccd144038a0454ba377648e18868e5ddc4c8))
* **cli:** draw a small AR mark in the terminal ([b7a313b](https://github.com/anyrouter-dev/cli/commit/b7a313b68e0bc4154f957351e59836fc7dbd23e7))
* **cli:** draw the AR mark on more terminals ([75e2a79](https://github.com/anyrouter-dev/cli/commit/75e2a79c83845cfaec0c8df446986a6d1e9be5ea))
* **cli:** gh-style auth command group ([47563d6](https://github.com/anyrouter-dev/cli/commit/47563d6e69b7d4b84067c151d329d7c31e361436))
* **cli:** interactive TUI for ar config ([b2c7d1a](https://github.com/anyrouter-dev/cli/commit/b2c7d1a5574fe7951d37ca9073e4271196215f9a))
* **cli:** native anyr 0.1.x with GitHub Releases install ([836f73f](https://github.com/anyrouter-dev/cli/commit/836f73f48c3a8d48d06c1c4ea49fef9f25e83694))
* **cli:** publish a beta from every main push ([ff71b81](https://github.com/anyrouter-dev/cli/commit/ff71b812b056e97eefd36914406db6c5c786752b))
* **cli:** spawn pi alongside claude, codex, and opencode ([56f28d0](https://github.com/anyrouter-dev/cli/commit/56f28d045c8aaa1ededc01e1155927134ffe0474))


### Bug Fixes

* **ci:** resolve release bench binary by absolute path ([4291216](https://github.com/anyrouter-dev/cli/commit/429121641f56e004f7c7fbba596ffbe7eccc2b81))
* **ci:** stamp the beta version after locked tests ([8308aeb](https://github.com/anyrouter-dev/cli/commit/8308aeb3bb168ca37cb399aebd8fe0d74561fe58))
* **cli:** drop install and docs lines from --help ([0707efb](https://github.com/anyrouter-dev/cli/commit/0707efb2a41be16d7634b892426d38fd2e9fb224))
* **cli:** keep Claude opus/sonnet/haiku aliases distinct ([f81e971](https://github.com/anyrouter-dev/cli/commit/f81e971bd75bd15a54aa9a3c2fbb7f7e07ae9257))
* **cli:** login then launcher; group help ([cd8ace1](https://github.com/anyrouter-dev/cli/commit/cd8ace1bd286ac8acdb0c54057ebea5f10258371))
* **cli:** login URL includes the code and opens the browser ([a5e1307](https://github.com/anyrouter-dev/cli/commit/a5e130768a4f7bcaa17c7429ea4dc3738d9a39ab))
* **cli:** open the TUI launcher on a bare terminal run ([0508692](https://github.com/anyrouter-dev/cli/commit/05086924c74529adbb8f098de35454603e86ad22))
* **cli:** print help using the invoked command name ([16c0f1f](https://github.com/anyrouter-dev/cli/commit/16c0f1f779ac3d0ca797cff64264acc8e29ad896))
* **cli:** render a clearer AR mark ([9a2d468](https://github.com/anyrouter-dev/cli/commit/9a2d46872b7b3474ede863b57b0e601be0ba7bfb))
* **cli:** use API key for keys CRUD; drop management-key requirement ([#2](https://github.com/anyrouter-dev/cli/issues/2)) ([53e9442](https://github.com/anyrouter-dev/cli/commit/53e94422996597d2b24d8a759bf0edfa0b1f943b))
* **cli:** wrap pi with AnyRouter key and endpoint ([90246a1](https://github.com/anyrouter-dev/cli/commit/90246a160212c915617bb8b804245179e1701b4d))
* **release:** lock release-please to always-bump-patch ([4b778a2](https://github.com/anyrouter-dev/cli/commit/4b778a242bb137fc0d6fe838950e546f05923153))
* **release:** show full release-please changelog on GitHub Releases ([454e6e3](https://github.com/anyrouter-dev/cli/commit/454e6e37e4da6a46d751d82f6a88e1ddd8257263))

## [0.1.9](https://github.com/anyrouter-dev/cli/compare/v0.1.8...v0.1.9) (2026-08-21)


### Features

* **cli:** auto-update in the background by default ([f3d5edd](https://github.com/anyrouter-dev/cli/commit/f3d5edd43c3185153af4ac88647670ea7f1faff2))
* **cli:** draw a small AR mark in the terminal ([b7a313b](https://github.com/anyrouter-dev/cli/commit/b7a313b68e0bc4154f957351e59836fc7dbd23e7))
* **cli:** draw the AR mark on more terminals ([75e2a79](https://github.com/anyrouter-dev/cli/commit/75e2a79c83845cfaec0c8df446986a6d1e9be5ea))
* **cli:** publish a beta from every main push ([ff71b81](https://github.com/anyrouter-dev/cli/commit/ff71b812b056e97eefd36914406db6c5c786752b))


### Bug Fixes

* **ci:** stamp the beta version after locked tests ([8308aeb](https://github.com/anyrouter-dev/cli/commit/8308aeb3bb168ca37cb399aebd8fe0d74561fe58))
* **cli:** drop install and docs lines from --help ([0707efb](https://github.com/anyrouter-dev/cli/commit/0707efb2a41be16d7634b892426d38fd2e9fb224))
* **cli:** open the TUI launcher on a bare terminal run ([0508692](https://github.com/anyrouter-dev/cli/commit/05086924c74529adbb8f098de35454603e86ad22))
* **cli:** render a clearer AR mark ([9a2d468](https://github.com/anyrouter-dev/cli/commit/9a2d46872b7b3474ede863b57b0e601be0ba7bfb))
* **cli:** use API key for keys CRUD; drop management-key requirement ([#2](https://github.com/anyrouter-dev/cli/issues/2)) ([53e9442](https://github.com/anyrouter-dev/cli/commit/53e94422996597d2b24d8a759bf0edfa0b1f943b))
* **release:** show full release-please changelog on GitHub Releases ([454e6e3](https://github.com/anyrouter-dev/cli/commit/454e6e37e4da6a46d751d82f6a88e1ddd8257263))

## [0.1.8](https://github.com/anyrouter-dev/cli/compare/v0.1.7...v0.1.8) (2026-08-20)

### Bug Fixes

* **cli:** login URL includes the code and opens the browser ([a5e1307](https://github.com/anyrouter-dev/cli/commit/a5e130768a4f7bcaa17c7429ea4dc3738d9a39ab))

## [0.1.7](https://github.com/anyrouter-dev/cli/compare/v0.1.6...v0.1.7) (2026-08-20)

### Bug Fixes

* **cli:** keep Claude opus/sonnet/haiku aliases distinct ([f81e971](https://github.com/anyrouter-dev/cli/commit/f81e971bd75bd15a54aa9a3c2fbb7f7e07ae9257))

## [0.1.6](https://github.com/anyrouter-dev/cli/compare/v0.1.5...v0.1.6) (2026-08-20)

### Bug Fixes

* **cli:** wrap pi with AnyRouter key and endpoint ([90246a1](https://github.com/anyrouter-dev/cli/commit/90246a160212c915617bb8b804245179e1701b4d))

## [0.1.5](https://github.com/anyrouter-dev/cli/compare/v0.1.4...v0.1.5) (2026-08-20)

### Features

* **cli:** interactive TUI for ar config ([b2c7d1a](https://github.com/anyrouter-dev/cli/commit/b2c7d1a5574fe7951d37ca9073e4271196215f9a))

## [0.1.4](https://github.com/anyrouter-dev/cli/compare/v0.1.3...v0.1.4) (2026-08-20)

### Features

* **cli:** gh-style auth command group ([47563d6](https://github.com/anyrouter-dev/cli/commit/47563d6e69b7d4b84067c151d329d7c31e361436))

## [0.1.3](https://github.com/anyrouter-dev/cli/compare/v0.1.2...v0.1.3) (2026-08-20)

### Bug Fixes

* **cli:** login then launcher; group help ([cd8ace1](https://github.com/anyrouter-dev/cli/commit/cd8ace1bd286ac8acdb0c54057ebea5f10258371))

## [0.1.2](https://github.com/anyrouter-dev/cli/compare/v0.1.1...v0.1.2) (2026-08-20)

### Bug Fixes

* **ci:** resolve release bench binary by absolute path ([4291216](https://github.com/anyrouter-dev/cli/commit/429121641f56e004f7c7fbba596ffbe7eccc2b81))

## [0.1.1](https://github.com/anyrouter-dev/cli/compare/v0.1.0...v0.1.1) (2026-08-20)

### Features

* **ci:** build every platform, bench size/startup, ship wasm demo ([532a1e0](https://github.com/anyrouter-dev/cli/commit/532a1e08f31a8bcd2567cc5db2269a30ff9f27e4))
* **cli:** device login, key/model switch, agent install, color TUI ([cc4cccd](https://github.com/anyrouter-dev/cli/commit/cc4cccd144038a0454ba377648e18868e5ddc4c8))
* **cli:** spawn pi alongside claude, codex, and opencode ([56f28d0](https://github.com/anyrouter-dev/cli/commit/56f28d045c8aaa1ededc01e1155927134ffe0474))

### Bug Fixes

* **cli:** print help using the invoked command name ([16c0f1f](https://github.com/anyrouter-dev/cli/commit/16c0f1f779ac3d0ca797cff64264acc8e29ad896))
* **release:** lock release-please to always-bump-patch ([4b778a2](https://github.com/anyrouter-dev/cli/commit/4b778a242bb137fc0d6fe838950e546f05923153))

## [0.1.0](https://github.com/anyrouter-dev/cli/releases/tag/v0.1.0) (2026-08-20)

### Features

* **cli:** native anyr 0.1.x with GitHub Releases install ([836f73f](https://github.com/anyrouter-dev/cli/commit/836f73f48c3a8d48d06c1c4ea49fef9f25e83694))
