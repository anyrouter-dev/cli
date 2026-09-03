# TUI launcher

The TUI launcher lets a user pick a coding agent first, then change **that** agent's model / account / key on the same home screen. Launch rows stay visible (configure does not replace them). `--dump-tui` prints a single ANSI-free frame for tests and pipes.

## Sub-features

- `menu-dump` prints one launcher/palette frame with `LAUNCH` rows (bindings beside each agent), `CONFIGURE` for the highlighted agent, and a `❯` input line.
- `menu-dump-empty` shows `install an agent…` when no agents are detected.
- `config-dump` prints grouped settings (`ACCOUNT`, `MODEL`, `AGENT`, `GENERAL`).
- `menu-quit` (PTY) leaves the interactive launcher with Escape without launching an agent.

## How to get to it (user POV)

- Run `anyr` or `anyr menu` on a TTY.
- Run `anyr menu --dump-tui` (or `ANYR_TUI_DUMP=1`) to print one frame and exit.
- Run `anyr config` on a TTY, or `anyr config --dump-tui`.
- Highlight an agent, then use `model…` / `account…` / `key…` on the same screen. Those rows apply to the highlighted agent.
- Type to filter (`cla`, `codex`, `model`), ↑↓ to move, ↵ to launch or switch, esc to quit.

## Driving it with control-anyr

Preconditions:

- `control-anyr doctor` is clean.
- Isolated config contains the fixture `default` profile.
- Helper sets `ANYR_AGENTS=claude,codex` unless a recipe overrides it.

- **Palette dump.** Run `control-anyr cli --out artifacts/tui-launcher/menu-dump.txt -- menu --dump-tui`. Exit code `0`. Stdout has no ESC (`\x1b`). Stdout contains `LAUNCH`, `claude`, `CONFIGURE`, `for claude` (or `for codex`), `model…`, `account…`, `key…`, `MORE`, `config…`, `❯`, `╭`, `╯`, and either `⚡` or `◆`. Agent rows include ` · `. Stdout does not contain `verify-fixture-not-a-real-key`. `LAUNCH` and `CONFIGURE` both appear (configure does not hide launch).
- **Empty agents.** Run `ANYR_AGENTS=none control-anyr cli --out artifacts/tui-launcher/menu-empty.txt -- menu --dump-tui` only if you temporarily export `ANYR_AGENTS=none` for that one command. Exit code `0`. Stdout contains `install an agent…` and `none detected`. Restore `ANYR_AGENTS=claude,codex` afterward (a new `doctor` / next `cli` uses the helper default).
- **Settings dump.** Run `control-anyr cli --out artifacts/tui-launcher/config-dump.txt -- config --dump-tui`. Exit code `0`. Stdout contains `ACCOUNT`, `MODEL`, `AGENT`, `GENERAL`, `auto-update`, and `update channel`. Secret substring `config-dump` / `verify-fixture-not-a-real-key` is absent.
- **Interactive pick-agent flow (optional).** Run `control-anyr pty start -- menu`, then `control-anyr pty capture --path artifacts/tui-launcher/pane.txt`. The pane shows `LAUNCH` and `CONFIGURE` together. Send Down to highlight the second agent if present; capture again as `pane-focus.txt` if you need proof configure retargets. Send Escape, then `control-anyr pty stop`. Do not press Enter on a launch row.
- **Proof.** Keep `menu-dump.txt` and `config-dump.txt`. They identify this binary's launcher and settings without attaching a fullscreen TTY.

## Gotchas

- `ANYR_AGENTS` overrides PATH detection. Leave it at `claude,codex` for the default dump; `none` is only for the empty-agents row.
- Dump mode is not a TTY session. Do not claim keybindings work from `--dump-tui` alone; use `pty` for filter typing / focus.
- Palette treats every printable character as filter input. Send Escape to quit (not `q`, which types into the query). Prefer `--dump-tui` unless the claim is interactive.
- Credits rows may show a placeholder when `ANYR_NO_CATALOG=1`. That is expected isolation, not a product outage.
- Do not select `claude` in the PTY. That path launches an agent.
- Do not dump the live model catalog from this flow. `anyrouter/auto` / most-used models are a separate job.
