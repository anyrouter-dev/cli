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

**beta** (every push to `main` publishes a GitHub prerelease):

```bash
curl -fsSL https://raw.githubusercontent.com/anyrouter-dev/cli/main/setup.sh | bash -s -- --channel beta
```

Docs and install: [anyrouter.dev/cli](https://anyrouter.dev/cli).

CI builds **linux x86_64/arm64**, **macOS x86_64/arm64**, **Windows x86_64**, and **wasm32**.
Every PR runs the full test suite (`cargo test --all-targets`) on each of those
platforms, plus LLVM coverage uploaded to [Codecov](https://codecov.io/gh/anyrouter-dev/cli).
Every PR and GitHub Release gets a size + startup report (`anyr --version` / `--help`).

Manual download of `https://github.com/anyrouter-dev/cli/releases/latest/download/anyr-linux-x86_64`
404s when the latest non-prerelease has no assets (v0.1.11 shipped empty). Use
`setup.sh` — it probes `/latest` and then picks a release that actually has
`anyr-linux-x86_64`. Direct tag URLs:

`https://github.com/anyrouter-dev/cli/releases/download/<tag>/anyr-linux-x86_64`

Hosted setup (same resolver):

```bash
curl -fsSL https://anyrouter.dev/setup.sh | bash
```

Add `~/.local/bin` to `PATH` if the installer says so.

### npm / npx

npm `@anyr/cli@latest` is still the previous JS CLI. The native line stays
**0.1.x** and publishes with `--tag next` until a human retags.

```bash
npx @anyr/cli@next --help
npx @anyr/cli@next login
npm install -g @anyr/cli@next
```

The npm package is a thin Node wrapper. It execs a shipped binary in
`binaries/` when present, otherwise it downloads the matching GitHub Release
asset (`anyr-linux-x86_64`, `anyr-linux-arm64`, `anyr-darwin-x86_64`, `anyr-darwin-arm64`, `anyr-windows-x86_64.exe`).

## Upgrade

```bash
anyr update                 # install latest for your current channel
anyr update --beta          # switch to beta (prereleases) and update
anyr update --stable        # switch to stable and update

anyr upgrade --check
anyr upgrade --check --channel beta   # one-shot, does not persist
```

Or re-run the installer / bump the npm `next` tag:

```bash
# curl, stable
curl -fsSL https://raw.githubusercontent.com/anyrouter-dev/cli/main/setup.sh | bash

# curl, beta
curl -fsSL https://raw.githubusercontent.com/anyrouter-dev/cli/main/setup.sh | bash -s -- --channel beta

# npm
npm install -g @anyr/cli@next
```

## Quick start

```bash
anyr                        # interactive TUI (TTY) — launch, model, key, credits
anyr --help                 # the everyday commands; anyr help --all for every command
anyr login                  # browser / device code / paste; key saved on this machine
anyr login --device         # headless / SSH
anyr usage                  # credit balance
anyr models use <id>        # persist default model
anyr keys list              # switch with: anyr keys use
anyr account use work       # switch saved profiles
anyr onboard impl           # paste-ready prompt for a coding agent
anyr plan                   # migration plan prompt (no code changes)
anyr fix                    # repair LLM wiring through AnyRouter
anyr claude --ok            # launch through the gateway (model: anyrouter/auto)
anyr claude --install --ok  # install Claude Code first if missing
anyr codex --ok
anyr opencode --ok
anyr pi --ok
anyr decision --model typesafe/jev \
  --state 'Help! My payouts have been failing for 3 days.' \
  --questions '{"is_urgent":{"type":"noul","instructions":"Does this convey urgency?"}}'
```

### For agents and scripts

`--json` (or `ANYR_OUTPUT=json`) works on every command: JSON on stdout where
the command has a JSON form, and every error as one JSON object on stderr.
Without a TTY there are no prompts, spinners or update notices.

```bash
anyr commands --json                     # the whole command tree
anyr chat "Reply with ok" --json         # {"id","model","content","finish_reason","usage"}
git diff | anyr chat "Review this" - --no-stream
anyr help agents                         # the guide, from docs/agents.md
```

Exit codes: 0 ok, 1 error, 2 usage, 3 not found, 4 auth, 5 rate limited.

### Dashboard from the terminal: `anyr api`

```bash
anyr api me                          # profile and balance
anyr api keys list --json
anyr api keys create ci-bot          # secret on stdout only: KEY=$(anyr api keys create x)
anyr api GET /credits                # raw request, like `gh api`
anyr api PATCH /keys/<hash> disabled=true limit:=25
anyr api POST /apps/explain/submissions --data @body.json     # body from a file
cat body.json | anyr api POST /apps/explain/submissions --data - -H 'X-Trace: 1'
```

Resources: `me credits keys logs presets aliases models providers dashboard hubs connections`.
Run `anyr api --help` for verbs. Piped output is TSV; pass `--json` for JSON.
Errors exit 2 (usage), 3 (404) or 4 (401/403).

### Publish to an app: `anyr publish`

Apps publish their own JSON Schema; the CLI knows no app by name. `publish` checks
your document against that schema locally (problems listed by field, exit 2, nothing
uploaded), then sends it and prints the URL.

```bash
anyr apps schema explain > schema.json        # what an agent must produce
<agent produces explain json> | anyr publish explain -
anyr publish explain fig.json --dry-run       # server checks it; nothing uploaded
anyr publish explain fig.json --visibility public --yes
anyr apps ls explain                          # what you published
anyr apps visibility explain <id> private
anyr apps rm explain <id> --yes
```

Visibility is `unlisted` by default (link works, not listed), `private` (only you) or
`public` (listed after review). Re-running the same document is safe: it carries a
stable `Idempotency-Key`, so the server returns the original instead of a copy.

### Publish a page: `anyr artifact`

```bash
anyr artifact publish report.html --title "Q3 report"   # prints the URL
anyr artifact publish ./site --visibility link          # folder upload
anyr artifact publish report.html --id <id> --label v2  # new version
anyr artifact ls                     # first page
anyr artifact ls --cursor <c>        # next page (cursor printed to stderr)
anyr artifact ls --all --json        # every page, one merged result
anyr artifact visibility <id> public --yes
anyr artifact rm <id> --yes
```

A file is sent inline; a folder uploads file by file and skips dotfiles,
`node_modules` and `.git` (limits: 5 MB per file, 50 files, 20 MB). `--id`
builds on the current version; if someone published first the command exits 2
and prints the current version. `public` asks for confirmation unless `--yes`.

### Scoped keys for CI: `anyr keys`

```bash
anyr keys create ci-bot --limit 25 --reset monthly --endpoints chat,embeddings --expires 30d
KEY=$(anyr keys create ci-bot --limit 5 --quiet)   # secret only on stdout, not saved
anyr keys update <hash> --limit 50 --disable       # --enable turns it back on
anyr keys show <hash>                              # limit, spent, endpoints, expiry
```

`--limit` is a USD cap, `--reset` its window (daily, weekly, monthly),
`--expires` takes `30d`, `12h`, `45m` or `2w`. `--endpoints` takes names (chat,
embeddings, moderations, rerank, documents, speech, decisions, mcp, me, relay,
models, keys, credits, logs, dashboard, trace). `chat`, `messages` and
`responses` share one grant on the server, so each covers all three.

### Request logs: `anyr logs`

```bash
anyr logs                            # latest 20 requests, newest first
anyr logs --status error --limit 50  # ok | error, limit 1..100
anyr logs --model z-ai/glm-4.7
anyr logs <id>                       # one request in detail
anyr logs sessions                   # grouped by session
anyr logs --json --status error | jq '.data | length'
```

Columns: time (local), model, provider, status (error code inline), tokens in
and out, cost, latency. `--json` prints the raw API body. A key without the
needed scope exits 4 and names the scope.

### Shared pool: `anyr pool`

```bash
anyr pool stats                      # providers, keys, donors, requests today, free coverage
anyr pool providers                  # per provider: keys, donors, recent masked donors
anyr pool free                       # backends the pool can serve for free right now
anyr pool relay                      # on-device relay pool: online devices and models
anyr pool providers --json | jq '.providers[].key_count'
```

All four read public endpoints, so no key is needed (one is sent if you have it).
`--json` prints the raw body; `stats --json` prints the three bodies under
`stats`, `providers` and `free_coverage`. `anyr pool` with no subcommand still
launches Poolside. Per-key status and last success are not exposed by the API.

### Saved routing presets: `anyr presets`

```sh
anyr presets                                   # slug, name, models, routing
anyr presets show coding
anyr presets create coding --model a/one,b/two --fallback --sort price
anyr presets edit coding --model a/one,b/two,c/three
anyr presets rm coding --yes
anyr claude --preset coding                    # launch with model @preset/coding
```

`--model` lists models in order (first is primary, the rest are the fallback chain); `--fallback` also allows paid fallback; `--sort` is `price`, `throughput`, `latency` or `exacto`. Presets need a management key (`ak_…`, scopes `read:presets` and `write:presets`) from <https://anyrouter.dev/settings/management-keys>: set `ANYROUTER_MANAGEMENT_KEY` or pass `--key ak_…`. `--preset` on a launch command checks the preset exists first and starts nothing if it does not; it cannot be combined with `--model`.

### MCP gateway: `anyr mcp`

```sh
anyr mcp url                 # https://anyrouter.dev/api/v1/mcp
anyr mcp add claude          # runs: claude mcp add --transport http anyrouter <url> --header ...
anyr mcp add codex           # edits ~/.codex/config.toml
anyr mcp add opencode        # edits ~/.config/opencode/opencode.json
anyr mcp add cursor          # edits ~/.cursor/mcp.json
anyr mcp add claude --print  # show the command, change nothing
anyr mcp connections         # upstream MCP connections (GET /v1/connections)
```

Config files keep their existing entries (JSON keys may be re-sorted), and a second run changes nothing. A commented `opencode.jsonc` or a config that is not plain JSON is left alone with an error; use `--print` and paste. The snippet and the written config read the key from `$ANYROUTER_API_KEY`; the key itself is only printed or written with `--show-key`. `add claude` without `--print` runs the `claude` binary (set `ANYROUTER_CLAUDE_PATH` to override) and stores the key in Claude Code's own config. An API key reaches the unscoped tools; pass a management key (`--key ak_…`) for key, credit and preset tools.

### One call: `anyr generation <id>`

```bash
anyr generation gen_abc123           # alias: anyr gen
anyr gen gen_abc123 --json | jq .total_cost
```

Shows model, provider, status, tokens, cost, latency, cache hit and time for
the response `id` (or `x-anyrouter-generation-id`). An unknown id prints the server message and exits 1.

### Plan, then implement: `anyr task`

```bash
anyr task "write a script that dedupes my downloads folder"
anyr task "refactor this CSV parser to stream" --plan-model openai/gpt-5.2 --do-model z-ai/glm-4.7-flash
anyr task "summarize RFC 8628 in five bullets" --yes --json | jq -r .result
```

A planning model writes a numbered plan, then an implementation model follows
it (two chat completions, plan text in the second prompt). Both stream. The
models and catalog prices are shown first and you confirm; scripts and CI must
pass `--yes`. `--json` skips streaming and prints `task`, `plan_model`,
`do_model`, `plan`, `result`, `usage` and `cost`.

### Hand a task to another agent: `anyr delegate`

```bash
anyr delegate --to codex "add unit tests for src/utils/slug.ts"
anyr delegate --to claude "explain this stack trace" --model deepseek/deepseek-v3
anyr delegate --to opencode "fix the lint errors in packages/ui" --yes
```

Runs the agent's non-interactive mode (`claude -p`, `codex exec`, `opencode run`)
with your AnyRouter key, base URL and model, streams its output and exits with
its exit code. You confirm first; scripts pass `--yes`. `--dry-run` prints the
command with the key redacted. One hop only: a delegated agent cannot delegate.

### Compare models: `anyr eval`

```bash
anyr eval "Summarize RFC 8628 in 3 bullets" --model openai/gpt-5-mini,z-ai/glm-4.7
anyr eval --file prompt.md --model a,b,c --max-tokens 400
anyr eval --prompts cases.jsonl --model a,b --json > results.jsonl   # {"prompt":...} per line
```

Sends one non-streaming chat completion per model (`--concurrency`, default 4)
and prints each answer (cut at 600 characters, `--full` for all), then a summary
table: model, provider that answered, latency, tokens in/out, cost, status.
`--json` prints one line per (prompt, model). Output follows `--model` order.
If a model fails it shows `error` and the others still print; the exit code is 1.

### Credit ledger: `anyr transactions`

```bash
anyr transactions                    # newest first, 20 rows
anyr transactions --type purchase
anyr transactions --limit 100
anyr transactions --json | jq '.data[0]'
```

Columns: date, type, amount (signed, $), balance after (when present), note.

### Embeddings: `anyr embed`

```bash
anyr embed "hello world"                         # default embedding model: dims + first 8 values
anyr embed --model baai/bge-base-en-v1.5 "hello"
anyr embed --file notes.txt                      # one input per line, one request
echo "hello" | anyr embed -                      # stdin
anyr embed --dimensions 256 "hello"
anyr embed --json hi | jq '.data[0].embedding | length'
```

Prints model, input count, dimensions, tokens, cost and a truncated vector per
input. `--json` prints the full response body.

### Bring your own provider keys: `anyr byok`

```bash
anyr byok                            # keys with a BYOK or DONATED badge
ANYR_BYOK_KEY=sk-... anyr byok add openai
anyr byok test openai                # one real call with the saved key
echo $KEY | anyr byok test openai --key-stdin   # check a key before saving it
anyr byok rm openai --yes            # delete the saved key (asks on a terminal)
anyr byok providers                  # providers that accept BYOK, ready/donated badge
```

`rm` without `--yes` and without a terminal exits 2. `test` exits 1 when the
provider rejects the key; the provider key is redacted from any error text.

Needs a management key (`ANYROUTER_MANAGEMENT_KEY=ak_...`) with `read:byok` and
`write:byok`. The provider key is read from `$ANYR_BYOK_KEY`, stdin or a hidden
prompt, never from argv.

`--donate` shares the key with the community pool. Caveat: server support is
still pending ([duyet/anyrouter#4220](https://github.com/duyet/anyrouter/issues/4220)),
so treat `--donate` as not yet effective.

### Serve this Mac's models: `anyr relay`

```bash
anyr relay                                  # where you stand + the next command
anyr relay start                            # serve, with a live status screen
anyr relay start --detach                   # serve in the background
anyr relay status [--json]                  # running?, connection, models, requests, tokens, pool
anyr relay logs --follow                    # background log
anyr relay stop                             # graceful stop (same as Ctrl-C)
anyr relay models [--json]                  # local models and which are served
anyr relay models disable apple/foundation-model
anyr relay devices [--json]                 # your paired devices: state, models, last seen, pool
anyr relay devices rename <id> "Studio Mac"
anyr relay devices pool <id> off            # stop sharing one device with the pool
anyr relay devices rm <id> --yes            # revoke a device (management key or session)
anyr relay models --devices                 # per model: devices, online, pool-shared
anyr relay pool off                         # stop donating idle capacity (saved)
anyr relay doctor                           # macOS, fm serve, Decider, network, auth
anyr relay unpair --yes                     # remove this device from your account
```

The relay detects the on-device models this Mac can serve:

- `apple/foundation-model`: macOS 26+ with Apple Intelligence on, via `fm serve`.
- `perplexity/pplx-decider-flash-v1.2`: the Decider for Mac app (port 8080).

The first `start` at a terminal opens a checklist (space toggles, enter
starts) and saves the answer; change it later with `anyr relay models
enable|disable <id>`. `--model <id>` (repeatable) and `--all` override the saved
choice for one run. Other local servers (Ollama, LM Studio, `--target <url>`)
are always served.

You don't need a second terminal. When a chosen model's server is installed
but not running, `start` launches it: `fm serve` as a child process, or the
Decider app with `open -g -a Decider`. It waits for each to answer, restarts
one that crashes (backoff 1s, 2s, 4s … up to 60s), and stops it when the relay
exits. The status screen marks these `(managed)`. A server you started
yourself shows as `(external)`; the relay never starts a second copy and
leaves it running on exit. `--no-launch` uses only servers already running,
and an explicit `--target` never launches anything.

When a model cannot be served, `start`, `relay models` and `relay doctor` say
which case it is and the next step: not installed (install the
Decider for Mac app into /Applications), installed but stopped (`start`
launches it, or open the app), not a Mac, or a Mac on a macOS that is too old (update it). The other models keep serving in
every case. `relay doctor --json` adds a `models` array with each model's
`state` (`running`, `stopped`, `not_installed`, `needs_mac`, `needs_newer_macos`) and `fix`.

At a terminal, `start` shows one live screen: connection, models, requests,
tokens and pool, with keys `p` (pool on/off), `r` (rescan local servers) and
`q` / Ctrl-C (stop). Piped, with `--quiet`, or with `TERM=dumb` it prints plain
log lines instead. `NO_COLOR` turns color off.

`--detach` runs the relay in the background on macOS and Linux. Its pidfile,
state and log live in `relay/` next to your AnyRouter config
(`~/.anyrouter/relay/`, or `$ANYROUTER_HOME/relay/`). `status`, `stop` and
`logs` work from any terminal, for background and foreground runs alike.

Pool sharing is on by default: when your own requests leave the device idle,
it serves other users' requests and earns you credits. `anyr relay pool off`
saves the opt-out and updates the paired device; `--pool` / `--no-pool`
override it for one run. See https://anyrouter.dev/pool and manage devices at
https://dash.anyrouter.dev/devices.

Exit codes: 0 ok, 1 failure (for `doctor`, a failed check), 2 usage error, 130
after Ctrl-C.

### Models: `anyr model list`

```bash
anyr model list                      # catalog ids usable with --model (alias of `anyr models`)
anyr model list --json
anyr model use <id>                  # persist the default model
anyr models --free                   # $0 models only (explicit $0 platform price)
anyr models --type embedding         # chat | embedding | decision
anyr models --owner nvidia --sort price   # sort: price | context | new
anyr models --search glm
anyr models show z-ai/glm-4.7        # pricing, context, modalities, providers, free route
anyr models show z-ai/glm-4.7 --json
```

Filtered lists print id, type, context, $/1M in, $/1M out and free; `--json`
prints the filtered list. `show` with an unknown id exits 1.

### Shell completion

```bash
anyr completion bash > ~/.local/share/bash-completion/completions/anyr
anyr completion zsh > "${fpath[1]}/_anyr"
anyr completion fish > ~/.config/fish/completions/anyr.fish
```

### Structured decisions

Decision models use AnyRouter's typed `POST /api/v1/decisions` endpoint, not a
chat completion. Pass the fields as flags, or pipe a JSON object containing
`model`, `state`, and `questions`:

```bash
anyr decision --model typesafe/jev \
  --state 'Help! My payouts have been failing for 3 days.' \
  --questions '{"is_urgent":{"type":"noul","instructions":"Does this convey urgency?"}}'

echo '{"model":"typesafe/jev","state":"A payout is failing","questions":{"urgent":{"type":"noul"}}}' \
  | anyr decision
```

The command uses the active AnyRouter profile (or `--key`, `--profile`, and
`--base-url`) and prints structured answers. `anyr claude`, `codex`, and other
chat-agent launchers refuse known Decisions models; use `anyr decision` for
`typesafe/jev`, `fastino/gliner2.5-multi-v1`, and `anyrouter/decision`.

On a terminal, bare `anyr` / `anyr menu` opens the Ratatui launcher by default.
Pipes and CI still get `--help` (or `anyr menu --dump-tui` for a plain frame).

Release size is tracked on every PR (`scripts/bench.py`): stripped
`anyr-linux-x86_64` must stay ≤ 4.0 MiB. Locally:

```bash
cargo build --release
strip -s target/release/anyr 2>/dev/null || true
python scripts/bench.py measure --bin target/release/anyr \
  --asset anyr-linux-x86_64 --kind native --out /tmp/anyr-bench.json
```

Interactive `menu` / `config` use a centered Ratatui dialog (GrokNight theme);
pickers stay full-pane. `anyr menu --dump-tui` prints one ANSI-free frame for CI.

## Passing flags to the agent

On the launchers (`claude`, `codex`, `grok`, `opencode`, `pi`, `pool`) you do
not need `--`. anyr takes its own flags (`--model`, `--key`, `--dry-run`,
`--yolo`, `--help`, ...) and forwards every other flag to the agent in order:

```bash
anyr codex exec --json "fix the tests"   # --json reaches codex
anyr claude --quiet -p "hi"              # --quiet reaches claude
anyr claude -- --model raw               # after `--`, everything is raw
```

`--help` is always anyr's. `--json` and `--quiet` belong to the agent; anyr's
own machine-output mode on a launcher is set with `ANYR_OUTPUT=json` and
`ANYR_QUIET=1`. The exception is `anyr pool --json`, a documented anyr flag.
Other commands (`anyr models --bogus`) still reject unknown flags with exit 2.

`--yolo` becomes each agent's own flag to skip permission prompts:

| Agent | `--yolo` becomes | Verified |
|-------|------------------|----------|
| claude | `--dangerously-skip-permissions` | yes |
| codex | `--dangerously-bypass-approvals-and-sandbox` | yes |
| grok | `--always-approve` | yes |
| opencode | `--auto` (root and after `run`, opencode 2.0.24) | yes |
| pi | none; `--yolo` is dropped with a note | yes |
| pool | `--unsafe-auto-allow` | no (docs only, not installed) |

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
