# Help and version

Help and version let a user see which `anyr` they have, which commands exist, and how to get per-command usage, without signing in or touching the network.

## Sub-features

- `help-root` prints grouped root help from `--help` and from a non-TTY empty argv.
- `help-command` prints per-command usage for implemented commands and honest stubs.
- `help-unknown` rejects an unknown command with a pointer back to `--help`.
- `version` prints the 0.1.x version line.

## How to get to it (user POV)

- Run `anyr --help` or `anyr -h`.
- Run `anyr` with no arguments when stdin is not a TTY.
- Run `anyr <command> --help` (for example `anyr auth --help`, `anyr claude --help`).
- Run `anyr --version` or `anyr -v`.
- Run an unknown token such as `anyr nope`.

## Driving it with control-anyr

Preconditions:

- `control-anyr doctor` is clean.
- Isolated `ANYROUTER_HOME` is in use.
- No live AnyRouter key is required.

- **Root help.** Show the grouped command list. Run `control-anyr cli --out artifacts/help-and-version/help.txt -- --help`. Exit code `0`. Stdout contains `CORE COMMANDS`, `LAUNCH`, `auth`, `claude`, `menu:`, `Sign in if needed`, and `▀█████████▄`. Stdout does not contain `setup.sh`, `Install:`, `npx @anyr/cli`, or `anyrouter.dev/docs/cli`.
- **Non-TTY empty argv.** Run `control-anyr cli --out artifacts/help-and-version/no-args.txt --`. With no extra args this is `anyr` and no TTY, so it must print the same grouped help and exit `0` (`Sign in if needed` and `CORE COMMANDS`).
- **Version.** Run `control-anyr cli --out artifacts/help-and-version/version.txt -- --version`. Exit code `0`. Stdout starts with `0.1.`.
- **Auth help.** Run `control-anyr cli --out artifacts/help-and-version/auth-help.txt -- auth --help`. Exit code `0`. Stdout contains `login`, `logout`, `status`, `token`, and `switch`.
- **Launch help.** Run `control-anyr cli --out artifacts/help-and-version/claude-help.txt -- claude --help`. Exit code `0`. Stdout contains `--dry-run` and `--ok`.
- **Stub help.** Run `control-anyr cli --out artifacts/help-and-version/task-help.txt -- task --help`. Exit code `0`. Stdout contains `not yet in the native CLI`.
- **Unknown command.** Run `control-anyr cli --out artifacts/help-and-version/unknown.txt -- nope`. Exit code is not `0`. Combined stdout/stderr contains `Unknown command` and `--help`.
- **Proof.** Keep `help.txt`, `version.txt`, and `help.txt.exit`. Together they show this checkout's `anyr` identifying itself as 0.1.x and listing the real command groups a user types.

## Gotchas

- On a TTY, bare `anyr` opens the launcher instead of help. `control-anyr cli` is not a TTY, so empty argv is the help path. Use `pty start -- menu` for the TTY path.
- Native `anyr --help` must name itself `anyr`, not `npx @anyr/cli`. Display name follows `ANYR_DISPLAY_BIN` / argv0 (`ar`, `anyrouter`).
- Root help must not tell an already-installed binary how to install (`setup.sh`).
- `task`, `chat`, `logs`, and similar stubs still exit `0` on `--help`. Assert the honest "not yet" sentence, not absence of help.
- Existing coverage lives in `tests/cli.rs` (`help_lists_login_claude_account_and_spawn_targets`, `version_matches_0_1`). Run `control-anyr test -- --test cli help_lists_login_claude_account_and_spawn_targets` as a complement, not as the user-path proof.
