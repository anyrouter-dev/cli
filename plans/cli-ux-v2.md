# anyr CLI UX v2 — research and plan

Status: phase 1 shipped on `feat/cli-api-completion`. Later phases are proposals.

## Research summary (clig.dev, gh, vercel, stripe, flyctl, cobra)

- Grammar: `noun verb` everywhere, the same verbs across nouns (`list get create update delete`), and short aliases (`ls show rm new`). Common paths get top-level shortcuts (`login`, `whoami`). A raw escape hatch (`gh api`) covers every endpoint before it has a typed command.
- Output: data goes to stdout and hints/errors to stderr. Tables on a TTY, TSV when piped, `--json` for a stable schema. Honour `NO_COLOR`. Never animate off a TTY.
- Errors: lines like `error: …` and `hint: <next command>`, did-you-mean on typos, and distinct exit codes (2 usage, 3 not found, 4 auth).
- Completion: one engine (`__complete`, cobra's model) behind thin bash/zsh/fish/pwsh shims. It is offline, never prompts and never writes to stderr.
- Wrappers: `--` passthrough, env-only injection, forward the exit code, nothing printed after the child starts, `--dry-run` to inspect.

## Command map

```
anyr                         launcher (account · model · agent)
anyr login | logout | whoami
anyr claude [--yolo] [--model m] [-- claude-args]     wrapper
anyr grok | codex | opencode | pi | pool              wrappers
anyr api <resource> <verb>   dashboard parity
anyr api [METHOD] /path k=v  raw passthrough (k:=json for typed)
anyr completion <shell>
```

`anyr api` resources (the server auth model is mapped in the research notes):

| resource | verbs | credential |
|---|---|---|
| me | get | sk-ar key |
| credits | get, transactions | sk-ar key |
| keys | list get create update revoke | sk-ar key |
| logs | list get sessions | sk-ar key |
| dashboard | get | sk-ar key |
| hubs, connections | list get | sk-ar key (read) |
| models, providers | list get | public |
| presets | list get create update delete | **ak_ management key** (`ANYROUTER_MANAGEMENT_KEY` or profile `management_key`) |
| aliases | list set | local Claude Code slots. **The server has no alias API.** |

## Phase 1 (done)

- [x] `anyr api` curated verbs plus raw passthrough, `--json`, TTY table / pipe TSV, exit codes 3 and 4.
- [x] `anyr completion bash|zsh|fish|powershell` and the hidden `anyr __complete`.
- [x] Did-you-mean on unknown commands (exit 2).
- [x] Help: `api` and `completion` in the root help and the command map.
- [x] Tests: a local fake HTTP server (`tests/api.rs`), plus a drift test that keeps the completion table in sync with dispatch.

## Phase 2 (proposed)

1. **Login mints a management key.** Server-side `/auth/cli/consent` already accepts `management_scopes`, but the exchange never returns `management_key`. Finishing it would let `anyr login` unlock presets and BYOK with no extra setup. This needs a server change.
2. **Restricted-key UX.** Keys with `allowed_endpoints` get a 403 on `/me` and `/keys`. Detect this at login, or in `whoami`, and offer to re-mint.
3. **Dynamic completion values.** Model ids from a cached catalog with about 30s TTL, plus key hashes and preset slugs. The cache must stay offline-safe.
4. **`--jq` filter** on `api` (gh parity), or document `| jq`.
5. **Ship completion files** in release archives and in the `setup.sh` install.
6. **The `ar` collision.** `~/.local/bin/ar` shadows binutils `ar`, which breaks any C or Rust build (`ring`) on a machine with anyr installed. Stop installing the `ar` alias by default, or rename it.

## Rating

See the PR description. Scores come from a fresh reviewer against the 10-criterion rubric in the research notes.
