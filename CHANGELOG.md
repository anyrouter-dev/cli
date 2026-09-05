# Changelog

This file is maintained automatically by [release-please](https://github.com/googleapis/release-please).

GitHub Releases use this file as the release notes (full history through that tag).

## [0.1.12](https://github.com/anyrouter-dev/cli/compare/v0.1.11...v0.1.12) (2026-09-05)


### Features

* **cli:** animate anyr update with from→to and channel ([#41](https://github.com/anyrouter-dev/cli/issues/41)) ([18be07f](https://github.com/anyrouter-dev/cli/commit/18be07f9c78db5bb90f8e0eccca84f2f189c8299))
* **cli:** Compact HUD launcher ([#51](https://github.com/anyrouter-dev/cli/issues/51)) ([3dd0d12](https://github.com/anyrouter-dev/cli/commit/3dd0d123aa8e6876e1e32e057e6214dd10a5c498))
* **cli:** compact HUD picker, examples-first help, status config ([#53](https://github.com/anyrouter-dev/cli/issues/53)) ([7c1bbd9](https://github.com/anyrouter-dev/cli/commit/7c1bbd9979c18439e3fe15ede3a2fd99d8d28292))
* **cli:** launch Claude right after install ([#54](https://github.com/anyrouter-dev/cli/issues/54)) ([b37c9c5](https://github.com/anyrouter-dev/cli/commit/b37c9c5cee920cfaf6caf07f4f5c2c3bab5be782))
* **cli:** spotlight chrome — examples-first help, brand orange, models table ([#50](https://github.com/anyrouter-dev/cli/issues/50)) ([7094017](https://github.com/anyrouter-dev/cli/commit/7094017c0e89545e9cfcae680982ab005886e783))
* **relay:** anyr relay start/pair — serve local models to the cloud ([0ee4699](https://github.com/anyrouter-dev/cli/commit/0ee4699572080fddfc49929322e99df73273bc48))
* **security:** read pasted API keys with terminal echo off ([#31](https://github.com/anyrouter-dev/cli/issues/31)) ([d6359c8](https://github.com/anyrouter-dev/cli/commit/d6359c8b14dbf3303b26940895591228e67a61bb)), closes [#21](https://github.com/anyrouter-dev/cli/issues/21)
* **security:** verify sha256 of downloaded release assets before replace ([#30](https://github.com/anyrouter-dev/cli/issues/30)) ([211c7bc](https://github.com/anyrouter-dev/cli/commit/211c7bcbcf71071d483aaa6790dfaba446e206a5)), closes [#20](https://github.com/anyrouter-dev/cli/issues/20)
* **tui:** AR-mark palette card, agent rows, settings polish ([866d7b0](https://github.com/anyrouter-dev/cli/commit/866d7b061c6214d1950a01df76c8ecc8fecf0eb1))
* **tui:** per-agent routing filters (exacto, tools, 1M ctx) ([#44](https://github.com/anyrouter-dev/cli/issues/44)) ([5ee9ce3](https://github.com/anyrouter-dev/cli/commit/5ee9ce396ffbd5875893fcf0fd042b508af56c7c))


### Bug Fixes

* **auth:** honor expires_in and bound transient network retries in device login ([#28](https://github.com/anyrouter-dev/cli/issues/28)) ([689b54f](https://github.com/anyrouter-dev/cli/commit/689b54fa98a52dd4c8e5dc7a0d742d167fa7bb96)), closes [#18](https://github.com/anyrouter-dev/cli/issues/18)
* **ci:** build darwin-x86_64 in the main matrix so beta installs work on Intel Macs ([#26](https://github.com/anyrouter-dev/cli/issues/26)) ([db13555](https://github.com/anyrouter-dev/cli/commit/db1355506269538e959278c9cb2193717424743c)), closes [#16](https://github.com/anyrouter-dev/cli/issues/16)
* **cli:** avoid GitHub Releases API 403 and empty /latest 404 ([#33](https://github.com/anyrouter-dev/cli/issues/33)) ([ca6e78e](https://github.com/anyrouter-dev/cli/commit/ca6e78eff4daa8b00191e92012fdc039545495d6)), closes [#32](https://github.com/anyrouter-dev/cli/issues/32)
* **cli:** fail loud on empty release assets; reject dummy test keys ([ba8b62f](https://github.com/anyrouter-dev/cli/commit/ba8b62f720f22ecbff1adba0b2a5c3a4d8f0a931))
* **cli:** stop the login re-reveal loop; persist profile across relogin ([358bcca](https://github.com/anyrouter-dev/cli/commit/358bcca0534653141bccbb01166721a57bf7bbb3))
* **pi:** write full catalog into models.json for /model picker ([b4941e2](https://github.com/anyrouter-dev/cli/commit/b4941e2945aad566636087e2a6cb22fc9c213b87))
* **relay:** never panic on multibyte request ids when naming worker threads ([#27](https://github.com/anyrouter-dev/cli/issues/27)) ([77adf3a](https://github.com/anyrouter-dev/cli/commit/77adf3aefd1b0a1c857035c0400d6032777219dc)), closes [#17](https://github.com/anyrouter-dev/cli/issues/17)
* **security:** write config atomically with 0700 dir / 0600 file ([#29](https://github.com/anyrouter-dev/cli/issues/29)) ([9ca7ac7](https://github.com/anyrouter-dev/cli/commit/9ca7ac7ea2a535f9059b42426b276d0e07f296dc)), closes [#19](https://github.com/anyrouter-dev/cli/issues/19)
* **tui:** bind model/account/key per coding agent ([#39](https://github.com/anyrouter-dev/cli/issues/39)) ([97d3370](https://github.com/anyrouter-dev/cli/commit/97d3370c144e3f53603f2b05c21e003577e8a8d5))
* **tui:** pin anyrouter/auto in the inline model picker ([#40](https://github.com/anyrouter-dev/cli/issues/40)) ([59b1f6a](https://github.com/anyrouter-dev/cli/commit/59b1f6aae4346db8d459de471fdbdde926457917))

## [0.1.11](https://github.com/anyrouter-dev/cli/compare/v0.1.10...v0.1.11) (2026-08-24)


### Features

* **cli:** ar update --beta|--stable switches channel and updates ([4b3db1b](https://github.com/anyrouter-dev/cli/commit/4b3db1b3ecf903805b9a52f12020a366da6fc6e6))
* **cli:** ar update --beta|--stable switches channel and updates ([dcf1b60](https://github.com/anyrouter-dev/cli/commit/dcf1b6035966b1c1f9eb7c7e0a554e51ce3e601e))
* **cli:** embed build time, shown in local timezone ([2fd669d](https://github.com/anyrouter-dev/cli/commit/2fd669dd8af37123a994932ce04f71024dd5e9d7))
* **cli:** looping launcher with config, login, and agent pick ([3b901c1](https://github.com/anyrouter-dev/cli/commit/3b901c1bc38782ee99825349020c71e9b9e5bc25))
* **cli:** model pin collapses alias slots; fable slot; build time ([256e0cb](https://github.com/anyrouter-dev/cli/commit/256e0cbcf5c84aaaf8eea419dbdbd155d5c67ec7))
* **cli:** pin model collapses claude alias slots; add fable slot ([96ef51a](https://github.com/anyrouter-dev/cli/commit/96ef51ae14ebe4471cfcf3539018bc21a3b2bc07))
* **cli:** Ratatui interactive TUI as the default launcher ([ee93b54](https://github.com/anyrouter-dev/cli/commit/ee93b54f0789dc9140db60dc305254c993db126c))
* **cli:** Ratatui launcher TUI as default ([aa7d4fb](https://github.com/anyrouter-dev/cli/commit/aa7d4fb3f22773840a394df1c912e5d58f19693d))
* **cli:** two-pane launcher with icons and cached credits ([26f9980](https://github.com/anyrouter-dev/cli/commit/26f9980649c2b4895b45f5844fd4cacabbc0167d))
* **tui:** centered dialog welcome launcher ([61a22ee](https://github.com/anyrouter-dev/cli/commit/61a22ee98e6d8130dce27f153497ebf0349c1ba5))
* **tui:** command-palette launcher with inline fallback ([ba25454](https://github.com/anyrouter-dev/cli/commit/ba25454fd665aed782c39322b3df81fd8a6a0f8d))
* **tui:** interactive config screen with identity and model memory ([#8](https://github.com/anyrouter-dev/cli/issues/8)) ([8a343d6](https://github.com/anyrouter-dev/cli/commit/8a343d67f5e3344f5b5102ec1f371b7d7de8013b))
* **tui:** show installed agents and per-agent settings tabs ([#10](https://github.com/anyrouter-dev/cli/issues/10)) ([8c7554c](https://github.com/anyrouter-dev/cli/commit/8c7554c84ddb7b23d9166f445e2921e3cedf0675))


### Bug Fixes

* **ci:** unblock stable release binaries ([99ba2bc](https://github.com/anyrouter-dev/cli/commit/99ba2bcb09a985249bf191e706e73d9a3e35af2c))
* **cli:** gate days_from_civil test to unix ([276098d](https://github.com/anyrouter-dev/cli/commit/276098d459d3d5576b7a1533738b96bab2312ba9))
* **cli:** label update current, latest, and status ([#14](https://github.com/anyrouter-dev/cli/issues/14)) ([629336a](https://github.com/anyrouter-dev/cli/commit/629336a041e1ab38e89e4ac26b4de6339dd7d5cf))
* **models:** catalog ids, Claude [1m] suffix, auto = most used ([#12](https://github.com/anyrouter-dev/cli/issues/12)) ([7fc97ba](https://github.com/anyrouter-dev/cli/commit/7fc97ba96748f02568a5cb20d23164166f5093f7))
* **pi:** pass catalog model ids and strip ANSI from --model ([#11](https://github.com/anyrouter-dev/cli/issues/11)) ([aad7641](https://github.com/anyrouter-dev/cli/commit/aad7641d4ac92fcfb4ba50aaf940c8488429fd37))
* **tui:** pick keys, accounts, and models from the CLI ([#9](https://github.com/anyrouter-dev/cli/issues/9)) ([ad95180](https://github.com/anyrouter-dev/cli/commit/ad951805eecb7ee3538c5c1844af8a8099194633))
* **wasm:** gate launcher palette entries behind native feature ([ddc7887](https://github.com/anyrouter-dev/cli/commit/ddc788721473a7a5074de457fb7b1dcada22a036))


### Performance Improvements

* **tui:** open the launcher without blocking on network or which ([#13](https://github.com/anyrouter-dev/cli/issues/13)) ([58fdb8b](https://github.com/anyrouter-dev/cli/commit/58fdb8b16dd8d6b532b57250db89dd00afdcd57e))

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
