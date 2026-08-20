//! WASM dry-run: real help, parse, and dry-run from this crate (size benches).
//! Network, spawn, and device login stay native-only.

use std::collections::BTreeMap;

use crate::help::{command_help, display_bin, root_help, set_invoked_bin};
use crate::http::{format_models_list, format_usage_report, CatalogModel};
use crate::key::mask_api_key;
use crate::parse::{get_string_flag, parse_cli_args};
use crate::spawn::{
    build_tool_env, canonical_tool, default_profile_for_env, effort_args_for, model_args_for,
    normalize_effort, provider_args_for, render_dry_run, resolve_tool, BuildToolEnvInput,
};
use crate::VERSION;

const DEMO_HINT: &str =
    "\n(browser demo — install the native CLI: curl -fsSL https://anyrouter.dev/setup.sh | bash)\n";

pub fn run_demo(line: &str) -> String {
    let tokens = tokenize(line);
    set_invoked_bin(invocation_bin(&tokens));
    let argv = strip_wrapper(tokens);
    match demo_argv(argv) {
        Ok(out) => out,
        Err(err) => format!("{err}\n"),
    }
}

fn invocation_bin(tokens: &[String]) -> String {
    if tokens.first().map(String::as_str) == Some("npx") {
        return "npx @anyr/cli".into();
    }
    tokens
        .first()
        .map(|s| display_bin(s))
        .unwrap_or_else(|| "anyr".into())
}

fn tokenize(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in line.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => cur.push(ch),
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            None => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn strip_wrapper(mut argv: Vec<String>) -> Vec<String> {
    if argv.first().map(String::as_str) == Some("npx") {
        argv.remove(0);
        if matches!(
            argv.first().map(String::as_str),
            Some("@anyr/cli" | "anyr" | "anyrouter")
        ) {
            argv.remove(0);
        }
    } else if let Some(first) = argv.first() {
        let name = std::path::Path::new(first)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(first);
        if matches!(name, "anyr" | "anyrouter" | "ar" | "cli") {
            argv.remove(0);
        }
    }
    argv
}

fn demo_argv(argv: Vec<String>) -> Result<String, String> {
    if argv.is_empty()
        || argv.iter().any(|a| a == "--help" || a == "-h") && argv.len() == 1
        || argv.first().map(String::as_str) == Some("help")
    {
        return Ok(root_help());
    }
    if argv.first().map(String::as_str) == Some("--version")
        || argv.first().map(String::as_str) == Some("-v")
    {
        return Ok(format!("{VERSION}\n"));
    }
    let parsed = parse_cli_args(&argv)?;
    let command = canonical_tool(&parsed.command);
    if parsed.flag_true("help")
        || parsed
            .passthrough
            .iter()
            .any(|a| a == "-h" || a == "--help")
        || command == "help"
    {
        return Ok(command_help(&parsed.command)
            .unwrap_or_else(root_help)
            .to_string());
    }
    match command {
        "claude" | "codex" | "grok" | "opencode" | "pool" | "pi" => demo_launch(command, &parsed),
        "models" => Ok(demo_models(parsed.flag_true("json"))),
        "usage" => Ok(demo_usage(parsed.flag_true("json"))),
        "whoami" | "status" => Ok(demo_whoami()),
        "login" | "setup" => Ok(format!(
            "{}\nDevice login and key storage run in the native CLI.\n  {} login --device\n  {} login --key sk-ar-…\n{DEMO_HINT}",
            command_help(command).unwrap_or_default(),
            crate::help::invoked_bin(),
            crate::help::invoked_bin()
        )),
        "upgrade" | "update" => Ok(format!(
            "{} upgrade checks GitHub Releases (anyrouter-dev/cli) on the native binary.\n{DEMO_HINT}",
            crate::help::invoked_bin()
        )),
        _ => {
            if let Some(help) = command_help(command) {
                Ok(format!("{help}{DEMO_HINT}"))
            } else {
                Err(format!(
                    "Unknown command \"{}\". Run \"{} --help\".{DEMO_HINT}",
                    parsed.command,
                    crate::help::invoked_bin()
                ))
            }
        }
    }
}

fn demo_launch(tool: &str, parsed: &crate::parse::ParsedArgs) -> Result<String, String> {
    let key = get_string_flag(&parsed.flags, "key").unwrap_or_else(|| "sk-ar-v1-demo-key".into());
    let profile = default_profile_for_env(None, Some(&key));
    let resolved = resolve_tool(None, tool)?;
    let model = get_string_flag(&parsed.flags, "model").unwrap_or_else(|| "auto".into());
    let effort = normalize_effort(get_string_flag(&parsed.flags, "effort").as_deref())?;
    let model_mode = if model == "auto" { "auto" } else { "concrete" };
    let env_map = build_tool_env(BuildToolEnvInput {
        tool_name: tool,
        tool: &resolved,
        profile: &profile,
        api_key: &key,
        model: &model,
        effort: effort.as_deref(),
        context_window: None,
        model_map: None,
    });
    let mut args = Vec::new();
    args.extend(effort_args_for(tool, effort.as_deref()));
    args.extend(provider_args_for(tool, &profile));
    args.extend(model_args_for(tool, &model, model_mode));
    args.extend(parsed.passthrough.clone());
    let out = render_dry_run(&resolved.command, &args, &env_map);
    Ok(format!(
        "{out}\n\n(dry-run in the browser demo — native `anyr {tool}` spawns the real agent)\n"
    ))
}

fn demo_models(json: bool) -> String {
    let models = [
        CatalogModel {
            id: "anthropic/claude-sonnet-4.6".into(),
            name: Some("Claude Sonnet 4.6".into()),
            owned_by: Some("anthropic".into()),
            context_length: Some(200_000),
        },
        CatalogModel {
            id: "openai/gpt-5.4-mini".into(),
            name: Some("GPT-5.4 mini".into()),
            owned_by: Some("openai".into()),
            context_length: Some(128_000),
        },
        CatalogModel {
            id: "z-ai/glm-4.7-flash".into(),
            name: Some("GLM 4.7 Flash".into()),
            owned_by: Some("z-ai".into()),
            context_length: Some(128_000),
        },
    ];
    let pinned = vec!["anthropic/claude-sonnet-4.6".into()];
    let (stdout, _) = format_models_list(&models, &pinned, Some("@preset/coding-stack"), json);
    format!("{stdout}{DEMO_HINT}")
}

fn demo_usage(json: bool) -> String {
    let credits = serde_json::json!({ "balance": 12.5, "total_usage": 3.2 });
    format!("{}{DEMO_HINT}", format_usage_report(&credits, json))
}

fn demo_whoami() -> String {
    let _env: BTreeMap<String, String> = BTreeMap::new();
    format!(
        "active account  default\napi_key         {}\ndefault_model   auto\n{DEMO_HINT}",
        mask_api_key(Some("sk-ar-v1-demo-key-value"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_is_real_root_help() {
        let out = run_demo("anyr --help");
        assert!(out.contains("auth"), "{out}");
        assert!(out.contains("anyr claude"), "{out}");
        assert!(out.contains("anyr auth login"), "{out}");
        assert!(out.contains("⠻⠟⠿"), "{out}");
        assert!(!out.contains("setup.sh"), "{out}");
        assert!(!out.contains("npx @anyr/cli"), "{out}");
    }

    #[test]
    fn help_follows_invoked_name() {
        let ar = run_demo("ar --help");
        assert!(ar.contains("ar claude"), "{ar}");
        assert!(ar.contains("ar <command>"), "{ar}");
        assert!(!ar.contains("npx @anyr/cli"), "{ar}");

        let npx = run_demo("npx @anyr/cli --help");
        assert!(npx.contains("npx @anyr/cli claude"), "{npx}");
        assert!(npx.contains("npx @anyr/cli <command>"), "{npx}");
    }

    #[test]
    fn version_matches_crate() {
        assert_eq!(run_demo("anyr --version").trim(), VERSION);
    }

    #[test]
    fn dry_run_redacts_key() {
        let out = run_demo("anyr claude --dry-run --key sk-ar-v1-secret-value");
        assert!(out.contains("ANTHROPIC_BASE_URL"), "{out}");
        assert!(!out.contains("sk-ar-v1-secret-value"), "{out}");
    }

    #[test]
    fn npx_wrapper_is_stripped() {
        let out = run_demo("npx @anyr/cli --version");
        assert_eq!(out.trim(), VERSION);
    }

    #[test]
    fn tokenize_respects_quotes() {
        assert_eq!(
            tokenize(r#"claude --model "z-ai/glm-4.7""#),
            vec!["claude", "--model", "z-ai/glm-4.7"]
        );
    }
}
