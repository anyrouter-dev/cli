//! `anyr delegate --to <agent> "<task>"`: run another coding agent once,
//! non-interactively, through the same gateway env as `anyr <agent>`.

use std::collections::BTreeMap;

use crate::cmd::dispatch::hint;
use crate::cmd::launch::{launch, Headless};
use crate::parse::{get_string_flag, ParsedArgs};

const USAGE_LINE: &str =
    "Usage: {bin} delegate --to claude|codex|opencode \"<task>\" [--model <id>] [--yes] [--dry-run]";

/// Set on the child so a delegated agent cannot delegate again.
pub(crate) const DEPTH_ENV: &str = "ANYROUTER_DELEGATE_DEPTH";
const FROM_ENV: &str = "ANYROUTER_DELEGATE_FROM";
const TARGETS: [&str; 3] = ["claude", "codex", "opencode"];

pub(crate) fn run_delegate(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let usage = || hint(USAGE_LINE);
    let task = parsed.passthrough.join(" ");
    let task = task.trim();
    let Some(to) = get_string_flag(&parsed.flags, "to") else {
        return Err(usage());
    };
    if !TARGETS.contains(&to.as_str()) {
        return Err(format!(
            "{}unknown agent \"{to}\" (one of: {})",
            usage(),
            TARGETS.join(", ")
        ));
    }
    if task.is_empty() {
        return Err(usage());
    }
    if env.get(DEPTH_ENV).is_some_and(|v| !v.is_empty()) {
        return Err("delegation is limited to one hop: this agent was already delegated to".into());
    }
    let mut extra_env = vec![(DEPTH_ENV.to_string(), "1".to_string())];
    if let Some(from) = get_string_flag(&parsed.flags, "from") {
        extra_env.push((FROM_ENV.to_string(), from));
    }
    let mut launch_args = parsed.clone();
    launch_args.passthrough.clear();
    launch_args.raw_from = None;
    launch(
        &to,
        &launch_args,
        env,
        Some(Headless {
            args: vec!["--print".into(), task.to_string()],
            extra_env,
        }),
    )
}
