//! Paste-ready prompts for coding agents to wire AnyRouter into a project.
//!
//! Mirrors `anyrouter/packages/lib/src/agent-prompts.ts` (implement / plan /
//! discover + migration sources) plus CLI-only `fix` and `deploy`. Keep
//! prompts self-contained and pointed at raw markdown docs.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::parse::ParsedArgs;
use crate::term;

pub const OPENAI_BASE_URL: &str = "https://anyrouter.dev/api/v1";
pub const ANTHROPIC_BASE_URL: &str = "https://anyrouter.dev/api";
pub const QUICKSTART_MD: &str = "https://docs.anyrouter.dev/getting-started/quickstart.md";
pub const LLMS_TXT: &str = "https://anyrouter.dev/llms.txt";
pub const LLMS_FULL_TXT: &str = "https://anyrouter.dev/llms-full.txt";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptVariant {
    pub id: &'static str,
    pub label: &'static str,
    pub tagline: &'static str,
    pub prompt: &'static str,
}

const IMPLEMENT: &str = concat!(
    "Goal: route this project's LLM calls through AnyRouter.\n",
    "\n",
    "AnyRouter is an OpenAI- and Anthropic-compatible gateway. Switching is a\n",
    "config change — keep the existing SDK, just repoint the base URL and key.\n",
    "Do not rewrite working code.\n",
    "\n",
    "Step 1: Read the contract.\n",
    "  Fetch the raw markdown and skim base URLs, auth header, the\n",
    "  `provider/model` id format, and streaming:\n",
    "    - Quickstart: https://docs.anyrouter.dev/getting-started/quickstart.md\n",
    "    - llms.txt:   https://anyrouter.dev/llms.txt\n",
    "\n",
    "Step 2: Confirm three things with me before editing.\n",
    "    1. Which SDK is in use? (OpenAI, Anthropic, LangChain, Vercel AI SDK, custom)\n",
    "    2. Which model id, in `provider/model` form? (e.g. openai/gpt-5.4-mini)\n",
    "    3. Is the key exported as ANYROUTER_API_KEY? If not, I'll export it —\n",
    "       never hardcode a key.\n",
    "\n",
    "Step 3: Repoint the client.\n",
    "  Change only the base URL and the key source; leave call sites alone.\n",
    "    - OpenAI-compatible SDK   → baseURL https://anyrouter.dev/api/v1\n",
    "    - Anthropic SDK / Claude  → baseURL https://anyrouter.dev/api\n",
    "  Then:\n",
    "    - Read the key from `ANYROUTER_API_KEY` (env, never inline).\n",
    "    - Write model ids in `provider/model` form.\n",
    "    - Reuse the existing client module if there is one; otherwise add a\n",
    "      single small file that exports the configured client.\n",
    "    - Preserve streaming and the current error handling.\n",
    "\n",
    "Step 4: Verify, then stop.\n",
    "  Add one smoke test that sends a short prompt and prints the reply, and\n",
    "  run it once. Pass = it returns (or streams) text. On 401/403, check that\n",
    "  `ANYROUTER_API_KEY` is set and starts with `sk-ar-`.\n",
    "\n",
    "Reference: https://docs.anyrouter.dev/getting-started/quickstart.md\n"
);

const PLAN: &str = concat!(
    "Plan a migration to AnyRouter for this project.\n",
    "Do NOT change any code yet — produce a written plan first.\n",
    "\n",
    "AnyRouter is an OpenAI- and Anthropic-compatible gateway. Switching is a\n",
    "config change, but I want you to audit the codebase before touching it.\n",
    "\n",
    "Step 1: Read the docs.\n",
    "    - Quickstart:  https://docs.anyrouter.dev/getting-started/quickstart.md\n",
    "    - llms.txt:    https://anyrouter.dev/llms.txt\n",
    "  Focus on base URLs, auth, model id format, streaming, and BYOK.\n",
    "\n",
    "Step 2: Audit the codebase.\n",
    "  Inventory every place that talks to an LLM:\n",
    "    - Which SDKs / clients are imported (openai, @anthropic-ai/sdk, ai, langchain, …)?\n",
    "    - Where are base URLs and API keys configured (env, config files, hardcoded)?\n",
    "    - Which model ids are used, and where?\n",
    "    - Where does streaming or tool-calling happen?\n",
    "    - Are there provider-specific request/response shapes that need adapting?\n",
    "\n",
    "Step 3: Draft the migration plan.\n",
    "  Produce a Markdown plan with:\n",
    "    - Files to change (path + the specific edit)\n",
    "    - New env vars (ANYROUTER_API_KEY, base URL overrides)\n",
    "    - Model id mapping (current → `provider/model` form)\n",
    "    - Risks (rate limits, streaming behavior, response shape diffs)\n",
    "    - Rollback strategy (one env var flip)\n",
    "    - Recommended order of changes\n",
    "\n",
    "Step 4: Stop and wait.\n",
    "  Show the plan. Ask me to approve before you make any changes.\n",
    "  Once approved, you can run the plan — but not before.\n",
    "\n",
    "Reference: https://docs.anyrouter.dev/getting-started/quickstart.md\n"
);

const DISCOVER: &str = concat!(
    "Help me pick which AnyRouter models to use in this project.\n",
    "\n",
    "AnyRouter routes to 150+ models across 28+ providers via one OpenAI-\n",
    "and Anthropic-compatible endpoint. I want a model recommendation grounded in\n",
    "what this codebase actually does — not a generic answer.\n",
    "\n",
    "Step 1: Read the docs and the models catalog.\n",
    "    - Quickstart:  https://docs.anyrouter.dev/getting-started/quickstart.md\n",
    "    - llms.txt:    https://anyrouter.dev/llms.txt\n",
    "    - Full docs:   https://anyrouter.dev/llms-full.txt\n",
    "  Note the model id format (`provider/model`) and the API surfaces\n",
    "  (chat, responses, embeddings, images, messages).\n",
    "\n",
    "Step 2: Read the codebase.\n",
    "  Figure out the LLM workload(s):\n",
    "    - Use case (chat, agents, RAG, code, vision, embeddings, image gen)?\n",
    "    - Latency vs quality vs cost priorities (visible in prompts, retries,\n",
    "      timeouts, caching)?\n",
    "    - Context window needs (long-doc summarization? short turns?)?\n",
    "    - Multimodal needs (images, audio)?\n",
    "    - Tool calling / structured output?\n",
    "\n",
    "Step 3: Recommend 2-3 models per workload.\n",
    "  For each option give:\n",
    "    - The `provider/model` id (exact, copy-pasteable)\n",
    "    - Why it fits (1-2 sentences)\n",
    "    - Trade-offs vs the other options (cost, latency, quality, context)\n",
    "  Prefer models that exist on AnyRouter today — verify against\n",
    "  https://anyrouter.dev/llms.txt.\n",
    "\n",
    "Step 4: Hand off.\n",
    "  If I pick options, ask whether to:\n",
    "    (a) just print the env var changes, or\n",
    "    (b) wire them in (then switch to the \"Set up AnyRouter\" prompt).\n",
    "\n",
    "Reference: https://docs.anyrouter.dev/getting-started/quickstart.md\n"
);

const FIX: &str = concat!(
    "Goal: fix this project's LLM wiring so requests succeed through AnyRouter.\n",
    "Do not rewrite working call sites — repair config, env, and model ids only.\n",
    "\n",
    "Step 1: Read the contract.\n",
    "    - Quickstart: https://docs.anyrouter.dev/getting-started/quickstart.md\n",
    "    - Errors:     https://anyrouter.dev/docs/guides/errors.md\n",
    "    - llms.txt:   https://anyrouter.dev/llms.txt\n",
    "\n",
    "Step 2: Diagnose from evidence.\n",
    "  Collect the failing status/body (401/403/404/429/5xx), the base URL in\n",
    "  use, the model id string, and whether `ANYROUTER_API_KEY` is set and\n",
    "  starts with `sk-ar-`. Never print the full key.\n",
    "\n",
    "Step 3: Apply the smallest fix.\n",
    "  Common repairs:\n",
    "    - OpenAI-compatible clients → https://anyrouter.dev/api/v1\n",
    "    - Anthropic SDK / Claude Code → https://anyrouter.dev/api\n",
    "    - Key from env `ANYROUTER_API_KEY` (never hardcoded)\n",
    "    - Model ids in `provider/model` form (verify against llms.txt)\n",
    "    - Preserve streaming and existing error handling\n",
    "\n",
    "Step 4: Smoke test once, then stop.\n",
    "  Send a short prompt. Pass = text returns (or streams). Summarize what\n",
    "  changed in one short paragraph.\n",
    "\n",
    "Reference: https://docs.anyrouter.dev/getting-started/quickstart.md\n"
);

const DEPLOY: &str = concat!(
    "Goal: ship AnyRouter credentials and base URL into this project's deploy / CI.\n",
    "Do not change application call sites unless env wiring is missing.\n",
    "\n",
    "Step 1: Read the contract.\n",
    "    - Quickstart: https://docs.anyrouter.dev/getting-started/quickstart.md\n",
    "    - Auth:       https://anyrouter.dev/docs/getting-started/authentication.md\n",
    "\n",
    "Step 2: Inventory runtime surfaces.\n",
    "  Find every place that needs LLM access in prod/staging/CI (env files,\n",
    "  secret stores, GitHub Actions, Cloudflare/Workers bindings, Docker,\n",
    "  serverless config). Note current key/env names and base URLs.\n",
    "\n",
    "Step 3: Wire secrets safely.\n",
    "  - Store the key as `ANYROUTER_API_KEY` (or map an existing secret to it).\n",
    "  - OpenAI-compatible → https://anyrouter.dev/api/v1\n",
    "  - Anthropic-compatible → https://anyrouter.dev/api\n",
    "  - Never commit keys. Prefer platform secrets / OIDC where available.\n",
    "  - Keep model ids in `provider/model` form.\n",
    "\n",
    "Step 4: Verify in the target environment.\n",
    "  Add or run one smoke check that hits the gateway and prints a short\n",
    "  reply. Document the required secrets in README or deploy docs.\n",
    "\n",
    "Reference: https://docs.anyrouter.dev/getting-started/quickstart.md\n"
);

const FROM_OPENAI: &str = concat!(
    "Goal: migrate this project from OpenAI to AnyRouter.\n",
    "\n",
    "AnyRouter is an OpenAI-compatible gateway — switching is a one-line config\n",
    "change. Keep the existing openai SDK, just repoint baseURL and swap the key.\n",
    "\n",
    "Step 1: Read the contract.\n",
    "  Fetch and skim base URLs, auth, model id format, and streaming:\n",
    "    - Quickstart: https://docs.anyrouter.dev/getting-started/quickstart.md\n",
    "    - llms.txt:   https://anyrouter.dev/llms.txt\n",
    "\n",
    "Step 2: Confirm before editing.\n",
    "  Tell me: which model id (in `provider/model` form, e.g. openai/gpt-4o)?\n",
    "  If unsure, suggest the closest match from llms.txt.\n",
    "\n",
    "Step 3: Repoint the client.\n",
    "  Change only:\n",
    "    - baseURL → https://anyrouter.dev/api/v1\n",
    "    - apiKey  → process.env.ANYROUTER_API_KEY  (never inline)\n",
    "  Write model ids as `openai/<model>` (e.g. openai/gpt-4o-mini).\n",
    "  Leave every call site untouched.\n",
    "\n",
    "Step 4: Smoke test.\n",
    "  Send one short prompt, print the reply. 401/403 → check ANYROUTER_API_KEY\n",
    "  starts with `sk-ar-`. Pass = done.\n",
    "\n",
    "Reference: https://docs.anyrouter.dev/getting-started/quickstart.md\n"
);

const FROM_ANTHROPIC: &str = concat!(
    "Goal: migrate this project from Anthropic SDK to AnyRouter.\n",
    "\n",
    "AnyRouter is Anthropic-SDK-compatible — switching is a one-line config change.\n",
    "Keep the existing @anthropic-ai/sdk, just repoint the baseURL and swap the key.\n",
    "\n",
    "Step 1: Read the contract.\n",
    "  Fetch and skim base URLs, auth, model id format, and streaming:\n",
    "    - Quickstart: https://docs.anyrouter.dev/getting-started/quickstart.md\n",
    "    - llms.txt:   https://anyrouter.dev/llms.txt\n",
    "\n",
    "Step 2: Confirm before editing.\n",
    "  Tell me: which model id (in `provider/model` form, e.g. anthropic/claude-sonnet-4-6)?\n",
    "\n",
    "Step 3: Repoint the client.\n",
    "  Change only:\n",
    "    - baseURL → https://anyrouter.dev/api\n",
    "    - apiKey  → process.env.ANYROUTER_API_KEY  (never inline)\n",
    "  Write model ids as `anthropic/<model>` (e.g. anthropic/claude-haiku-4.5-20251001).\n",
    "  Leave every call site untouched.\n",
    "\n",
    "Step 4: Smoke test.\n",
    "  Send one short prompt, print the reply. 401/403 → check ANYROUTER_API_KEY\n",
    "  starts with `sk-ar-`. Pass = done.\n",
    "\n",
    "Reference: https://docs.anyrouter.dev/getting-started/quickstart.md\n"
);

const FROM_OPENROUTER: &str = concat!(
    "Goal: migrate this project from OpenRouter to AnyRouter.\n",
    "\n",
    "AnyRouter is OpenAI-compatible — the same interface OpenRouter uses.\n",
    "Switching is a one-line baseURL change; all model ids stay in `provider/model` form.\n",
    "\n",
    "Step 1: Read the contract.\n",
    "  Fetch and skim base URLs, auth, model id format, and streaming:\n",
    "    - Quickstart: https://docs.anyrouter.dev/getting-started/quickstart.md\n",
    "    - llms.txt:   https://anyrouter.dev/llms.txt\n",
    "\n",
    "Step 2: Confirm before editing.\n",
    "  List the OpenRouter model ids in use. I'll verify them against llms.txt and\n",
    "  give you the AnyRouter equivalents (same `provider/model` format, minor id diffs).\n",
    "\n",
    "Step 3: Repoint the client.\n",
    "  Change only:\n",
    "    - baseURL → https://anyrouter.dev/api/v1\n",
    "    - apiKey  → process.env.ANYROUTER_API_KEY  (never inline)\n",
    "  Update model ids as needed (check llms.txt for exact strings).\n",
    "  Leave every call site untouched.\n",
    "\n",
    "Step 4: Smoke test.\n",
    "  Send one short prompt, print the reply. 401/403 → check ANYROUTER_API_KEY\n",
    "  starts with `sk-ar-`. Pass = done.\n",
    "\n",
    "Reference: https://docs.anyrouter.dev/getting-started/quickstart.md\n"
);

const FROM_CLAUDE_CODE: &str = concat!(
    "Goal: route this Claude Code (or MCP-based) project through AnyRouter.\n",
    "\n",
    "AnyRouter exposes a full Anthropic-compatible `/api` endpoint — just repoint\n",
    "the SDK and keep your existing tools, streaming, and system prompts unchanged.\n",
    "\n",
    "Step 1: Read the contract.\n",
    "  Fetch and skim base URLs, auth, model id format, and streaming:\n",
    "    - Quickstart: https://docs.anyrouter.dev/getting-started/quickstart.md\n",
    "    - llms.txt:   https://anyrouter.dev/llms.txt\n",
    "\n",
    "Step 2: Audit LLM touchpoints.\n",
    "  Find every place the Anthropic SDK (or Claude API directly) is called.\n",
    "  Note which model ids are used.\n",
    "\n",
    "Step 3: Repoint the client.\n",
    "  Change only:\n",
    "    - baseURL → https://anyrouter.dev/api\n",
    "    - apiKey  → process.env.ANYROUTER_API_KEY  (never inline)\n",
    "  Write model ids as `anthropic/<model>` (e.g. anthropic/claude-sonnet-4-6).\n",
    "  Leave tool definitions, streaming setup, and system prompts untouched.\n",
    "\n",
    "Step 4: Smoke test.\n",
    "  Send one short prompt, print the reply. 401/403 → check ANYROUTER_API_KEY\n",
    "  starts with `sk-ar-`. Pass = done.\n",
    "\n",
    "Reference: https://docs.anyrouter.dev/getting-started/quickstart.md\n"
);

const FROM_HERMES: &str = concat!(
    "Goal: run my Hermes / OpenClaw coding agent through AnyRouter.\n",
    "\n",
    "Step 1: Read https://docs.anyrouter.dev/getting-started/quickstart.md.\n",
    "Step 2: In the agent's model-provider config, set the OpenAI-compatible\n",
    "  endpoint to https://anyrouter.dev/api/v1 with the key from ANYROUTER_API_KEY.\n",
    "Step 3: Set the model to `anyrouter/hermes` — AnyRouter's auto-router that\n",
    "  picks the best agentic model per request — or any `provider/model` id\n",
    "  from https://anyrouter.dev/llms.txt.\n",
    "Step 4: Start the agent, run one short task, and confirm requests appear at\n",
    "  https://anyrouter.dev/dashboard/logs.\n"
);

/// Core + migration variants shown in the interactive picker (display order).
pub const VARIANTS: &[PromptVariant] = &[
    PromptVariant {
        id: "impl",
        label: "Set up AnyRouter",
        tagline: "Wire AnyRouter into an existing project.",
        prompt: IMPLEMENT,
    },
    PromptVariant {
        id: "plan",
        label: "Plan a migration",
        tagline: "Audit the codebase and draft a migration plan — no changes yet.",
        prompt: PLAN,
    },
    PromptVariant {
        id: "fix",
        label: "Fix LLM wiring",
        tagline: "Repair base URL, key, and model ids when requests fail.",
        prompt: FIX,
    },
    PromptVariant {
        id: "deploy",
        label: "Deploy / CI secrets",
        tagline: "Wire ANYROUTER_API_KEY and base URL into prod and CI.",
        prompt: DEPLOY,
    },
    PromptVariant {
        id: "discover",
        label: "Pick the right models",
        tagline: "Recommend models based on what this project actually does.",
        prompt: DISCOVER,
    },
    PromptVariant {
        id: "openai",
        label: "From OpenAI",
        tagline: "Migrate an OpenAI SDK project.",
        prompt: FROM_OPENAI,
    },
    PromptVariant {
        id: "anthropic",
        label: "From Anthropic",
        tagline: "Migrate an Anthropic SDK project.",
        prompt: FROM_ANTHROPIC,
    },
    PromptVariant {
        id: "openrouter",
        label: "From OpenRouter",
        tagline: "Swap base URL; keep provider/model ids.",
        prompt: FROM_OPENROUTER,
    },
    PromptVariant {
        id: "claude-code",
        label: "Claude Code",
        tagline: "Point Claude Code / Anthropic tools at AnyRouter.",
        prompt: FROM_CLAUDE_CODE,
    },
    PromptVariant {
        id: "hermes",
        label: "Hermes / OpenClaw",
        tagline: "Run Hermes or OpenClaw through the gateway.",
        prompt: FROM_HERMES,
    },
];

/// Normalize user aliases (`implement` → `impl`, etc.).
pub fn resolve_mode(raw: &str) -> Option<&'static PromptVariant> {
    let lower = raw.trim().to_ascii_lowercase();
    let key = match lower.as_str() {
        "implement" | "implementation" | "setup" => "impl",
        "migrate" => "plan",
        other => other,
    };
    VARIANTS.iter().find(|v| v.id == key)
}

pub fn usage_hint(bin: &str) -> String {
    format!(
        "\
{bin} onboard — paste-ready prompts for coding agents

USAGE
  {bin} onboard                 Interactive pick (TTY)
  {bin} onboard <mode>          Print the prompt
  {bin} onboard cp [mode]       Copy to clipboard (or print + hint)
  {bin} impl | plan | fix | deploy | cp

MODES
  impl        Set up AnyRouter (alias: implement)
  plan        Plan a migration (no code changes)
  fix         Repair LLM wiring through AnyRouter
  deploy      Wire secrets into deploy / CI
  discover    Recommend models for this project
  openai | anthropic | openrouter | claude-code | hermes

FLAGS
  --json      Emit {{\"mode\",\"label\",\"prompt\"}}
  --copy      Copy instead of printing (same as `cp`)
  -h, --help  Show this help
"
    )
}

fn want_copy(parsed: &ParsedArgs, command: &str, mode_args: &[String]) -> bool {
    parsed.flag_true("copy")
        || command == "cp"
        || mode_args.first().map(|s| s.as_str()) == Some("cp")
}

fn mode_token(command: &str, mode_args: &[String]) -> Option<String> {
    match command {
        "onboard" => {
            let mut args = mode_args.iter().map(|s| s.as_str());
            match args.next() {
                Some("cp") => args.next().map(|s| s.to_string()),
                Some(other) => Some(other.to_string()),
                None => None,
            }
        }
        "cp" => mode_args.first().cloned(),
        "impl" | "plan" | "fix" | "deploy" => Some(command.to_string()),
        _ => None,
    }
}

fn pick_variant_interactive() -> Result<&'static PromptVariant, String> {
    let items: Vec<String> = VARIANTS
        .iter()
        .map(|v| format!("{} — {}", v.id, v.label))
        .collect();
    let idx = term::pick("Agent onboard prompt", &items, Some(0))?;
    Ok(&VARIANTS[idx])
}

fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let candidates: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("pbcopy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
        ("clip.exe", &[]),
    ];
    for (bin, args) in candidates {
        let Ok(mut child) = Command::new(bin)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            if stdin.write_all(text.as_bytes()).is_err() {
                continue;
            }
        }
        if child.wait().map(|s| s.success()).unwrap_or(false) {
            return Ok(());
        }
    }
    Err("No clipboard tool found (tried wl-copy, pbcopy, xclip, xsel, clip.exe).".into())
}

fn emit(variant: &PromptVariant, json: bool, copy: bool) -> Result<i32, String> {
    if json {
        let payload = serde_json::json!({
            "mode": variant.id,
            "label": variant.label,
            "tagline": variant.tagline,
            "prompt": variant.prompt,
        });
        println!("{payload}");
        return Ok(0);
    }
    if copy {
        match copy_to_clipboard(variant.prompt) {
            Ok(()) => {
                eprintln!(
                    "{} copied ({})",
                    term::accent(variant.id),
                    term::dim(variant.label)
                );
                return Ok(0);
            }
            Err(err) => {
                eprintln!("{err}");
                eprintln!("{}", term::dim("Printing prompt instead:"));
            }
        }
    }
    print!("{}", variant.prompt);
    if term::is_interactive() && !copy {
        eprintln!(
            "\n{}  {}  ·  pipe or `{bin} onboard cp {id}`",
            term::dim("Paste into your coding agent."),
            term::dim(variant.tagline),
            bin = crate::help::invoked_bin(),
            id = variant.id,
        );
    }
    Ok(0)
}

/// Entry for `onboard` / `impl` / `plan` / `fix` / `deploy` / `cp`.
pub fn run(command: &str, parsed: &ParsedArgs) -> Result<i32, String> {
    let mode_args = &parsed.passthrough;
    let copy = want_copy(parsed, command, mode_args);
    let json = parsed.flag_true("json");

    let variant = if let Some(token) = mode_token(command, mode_args) {
        resolve_mode(&token).ok_or_else(|| {
            format!(
                "Unknown onboard mode \"{token}\". Run \"{} onboard --help\".",
                crate::help::invoked_bin()
            )
        })?
    } else if term::is_interactive() {
        pick_variant_interactive()?
    } else {
        return Err(format!(
            "Specify a mode (e.g. impl, plan, fix). Run \"{} onboard --help\".",
            crate::help::invoked_bin()
        ));
    };

    emit(variant, json, copy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{parse_cli_args, FlagValue};
    use std::collections::HashMap;

    #[test]
    fn resolve_aliases() {
        assert_eq!(resolve_mode("implement").unwrap().id, "impl");
        assert_eq!(resolve_mode("IMPL").unwrap().id, "impl");
        assert_eq!(resolve_mode("plan").unwrap().id, "plan");
        assert_eq!(resolve_mode("fix").unwrap().id, "fix");
        assert_eq!(resolve_mode("deploy").unwrap().id, "deploy");
        assert_eq!(resolve_mode("claude-code").unwrap().id, "claude-code");
        assert!(resolve_mode("nope").is_none());
    }

    #[test]
    fn prompts_mention_contract() {
        for v in VARIANTS {
            assert!(
                v.prompt.contains("ANYROUTER_API_KEY") || v.id == "discover" || v.id == "hermes",
                "{} missing key hint",
                v.id
            );
            assert!(
                v.prompt.contains(OPENAI_BASE_URL)
                    || v.prompt.contains(ANTHROPIC_BASE_URL)
                    || v.prompt.contains(QUICKSTART_MD),
                "{} missing base URL / quickstart",
                v.id
            );
        }
        let impl_p = resolve_mode("impl").unwrap().prompt;
        assert!(impl_p.contains(OPENAI_BASE_URL));
        assert!(impl_p.contains(ANTHROPIC_BASE_URL));
        assert!(impl_p.contains("ANYROUTER_API_KEY"));
        let plan_p = resolve_mode("plan").unwrap().prompt;
        assert!(plan_p.to_ascii_lowercase().contains("do not change"));
    }

    #[test]
    fn mode_token_from_aliases() {
        let parsed = parse_cli_args(["onboard", "impl"]).unwrap();
        assert_eq!(
            mode_token("onboard", &parsed.passthrough).as_deref(),
            Some("impl")
        );
        let parsed = parse_cli_args(["onboard", "cp", "plan"]).unwrap();
        assert_eq!(
            mode_token("onboard", &parsed.passthrough).as_deref(),
            Some("plan")
        );
        assert!(want_copy(&parsed, "onboard", &parsed.passthrough));
        let parsed = parse_cli_args(["fix"]).unwrap();
        assert_eq!(
            mode_token("fix", &parsed.passthrough).as_deref(),
            Some("fix")
        );
        let mut flags = HashMap::new();
        flags.insert("copy".into(), FlagValue::Bool(true));
        let with_flag = ParsedArgs {
            command: "onboard".into(),
            flags,
            passthrough: vec!["deploy".into()],
        };
        assert!(want_copy(&with_flag, "onboard", &with_flag.passthrough));
    }

    #[test]
    fn emit_json_shape() {
        let v = resolve_mode("impl").unwrap();
        let payload = serde_json::json!({
            "mode": v.id,
            "label": v.label,
            "tagline": v.tagline,
            "prompt": v.prompt,
        });
        assert_eq!(payload["mode"], "impl");
        assert!(payload["prompt"]
            .as_str()
            .unwrap()
            .contains("ANYROUTER_API_KEY"));
    }
}
