use std::collections::{BTreeMap, HashMap};
use std::process::{Command, Stdio};

use crate::config::{Profile, YamlValue, DEFAULT_BASE_URL, DEFAULT_PRESET, DEFAULT_TIMEOUT_MS};

const REASONING_LEVELS: &[&str] = &["minimal", "low", "medium", "high", "xhigh", "max"];
const CLAUDE_LEVELS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const CODEX_LEVELS: &[&str] = &["minimal", "low", "medium", "high"];
const CLAUDE_EFFORT_TOKENS: &[(&str, i64)] = &[
    ("low", 2048),
    ("medium", 8192),
    ("high", 24000),
    ("xhigh", 32000),
    ("max", 32000),
];

#[derive(Debug, Clone, PartialEq)]
pub struct ToolConfig {
    pub command: String,
    pub base_url_env: String,
    pub auth_env: String,
    pub model_env: Option<String>,
    pub base_suffix: String,
    pub enable_gateway_model_discovery: bool,
    pub shadow_env: Option<String>,
    pub extra: BTreeMap<String, YamlValue>,
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            base_url_env: String::new(),
            auth_env: String::new(),
            model_env: None,
            base_suffix: String::new(),
            enable_gateway_model_discovery: false,
            shadow_env: None,
            extra: BTreeMap::new(),
        }
    }
}

impl ToolConfig {
    pub fn merge(&mut self, over: &ToolConfig) {
        if !over.command.is_empty() {
            self.command = over.command.clone();
        }
        if !over.base_url_env.is_empty() {
            self.base_url_env = over.base_url_env.clone();
        }
        if !over.auth_env.is_empty() {
            self.auth_env = over.auth_env.clone();
        }
        if over.model_env.is_some() {
            self.model_env = over.model_env.clone();
        }
        self.base_suffix = over.base_suffix.clone();
        self.enable_gateway_model_discovery = over.enable_gateway_model_discovery;
        if over.shadow_env.is_some() {
            self.shadow_env = over.shadow_env.clone();
        }
        for (k, v) in &over.extra {
            self.extra.insert(k.clone(), v.clone());
        }
    }

    pub fn from_yaml(map: &BTreeMap<String, YamlValue>) -> Self {
        let mut tool = ToolConfig::default();
        for (key, value) in map {
            match key.as_str() {
                "command" => tool.command = value.as_string_lossy(),
                "base_url_env" => tool.base_url_env = value.as_string_lossy(),
                "auth_env" => tool.auth_env = value.as_string_lossy(),
                "model_env" => {
                    let s = value.as_string_lossy();
                    tool.model_env = if s.is_empty() || s == "null" {
                        None
                    } else {
                        Some(s)
                    };
                }
                "base_suffix" => tool.base_suffix = value.as_string_lossy(),
                "enable_gateway_model_discovery" => {
                    tool.enable_gateway_model_discovery = matches!(value, YamlValue::Bool(true))
                        || value.as_string_lossy() == "true"
                }
                "shadow_env" => {
                    let s = value.as_string_lossy();
                    tool.shadow_env = if s.is_empty() || s == "null" {
                        None
                    } else {
                        Some(s)
                    };
                }
                _ => {
                    tool.extra.insert(key.clone(), value.clone());
                }
            }
        }
        tool
    }

    pub fn to_yaml_lines(&self) -> Vec<String> {
        vec![
            format!("    command: {}", self.command),
            format!("    base_url_env: {}", self.base_url_env),
            format!("    auth_env: {}", self.auth_env),
            format!(
                "    model_env: {}",
                self.model_env.as_deref().unwrap_or("null")
            ),
            format!(
                "    base_suffix: {}",
                if self.base_suffix.is_empty() {
                    "\"\"".into()
                } else {
                    self.base_suffix.clone()
                }
            ),
            format!(
                "    enable_gateway_model_discovery: {}",
                self.enable_gateway_model_discovery
            ),
            format!(
                "    shadow_env: {}",
                self.shadow_env.as_deref().unwrap_or("null")
            ),
        ]
    }
}

fn builtin(id: &str) -> Option<ToolConfig> {
    Some(match id {
        "claude" => ToolConfig {
            command: "claude".into(),
            base_url_env: "ANTHROPIC_BASE_URL".into(),
            auth_env: "ANTHROPIC_AUTH_TOKEN".into(),
            model_env: Some("ANTHROPIC_MODEL".into()),
            base_suffix: String::new(),
            enable_gateway_model_discovery: true,
            shadow_env: Some("ANTHROPIC_API_KEY".into()),
            extra: BTreeMap::new(),
        },
        "codex" => ToolConfig {
            command: "codex".into(),
            base_url_env: "OPENAI_BASE_URL".into(),
            auth_env: "OPENAI_API_KEY".into(),
            model_env: Some("OPENAI_MODEL".into()),
            base_suffix: "/v1".into(),
            enable_gateway_model_discovery: false,
            shadow_env: Some("OPENAI_API_KEY".into()),
            extra: BTreeMap::new(),
        },
        "grok" => ToolConfig {
            command: "grok".into(),
            base_url_env: "GROK_MODELS_BASE_URL".into(),
            auth_env: "GROK_CODE_XAI_API_KEY".into(),
            model_env: None,
            base_suffix: "/v1".into(),
            enable_gateway_model_discovery: false,
            shadow_env: None,
            extra: BTreeMap::new(),
        },
        "opencode" => ToolConfig {
            command: "opencode".into(),
            base_url_env: "OPENAI_BASE_URL".into(),
            auth_env: "OPENAI_API_KEY".into(),
            model_env: Some("OPENAI_MODEL".into()),
            base_suffix: "/v1".into(),
            enable_gateway_model_discovery: false,
            shadow_env: None,
            extra: BTreeMap::new(),
        },
        "pool" => ToolConfig {
            command: "pool".into(),
            base_url_env: "POOLSIDE_STANDALONE_BASE_URL".into(),
            auth_env: "POOLSIDE_API_KEY".into(),
            model_env: Some("POOLSIDE_STANDALONE_MODEL".into()),
            base_suffix: "/v1".into(),
            enable_gateway_model_discovery: false,
            shadow_env: Some("OPENAI_API_KEY".into()),
            extra: BTreeMap::new(),
        },
        "pi" => ToolConfig {
            command: "pi".into(),
            base_url_env: "OPENAI_BASE_URL".into(),
            auth_env: "ANYROUTER_API_KEY".into(),
            model_env: None,
            base_suffix: "/v1".into(),
            enable_gateway_model_discovery: false,
            shadow_env: None,
            extra: BTreeMap::new(),
        },
        _ => return None,
    })
}

pub fn create_default_tools() -> BTreeMap<String, ToolConfig> {
    ["claude", "codex", "grok", "opencode", "pool", "pi"]
        .into_iter()
        .filter_map(|id| builtin(id).map(|t| (id.to_string(), t)))
        .collect()
}

pub fn canonical_tool(name: &str) -> &str {
    match name {
        "cc" => "claude",
        "poolside" => "pool",
        "status" => "whoami",
        other => other,
    }
}

pub fn resolve_tool(
    config: Option<&crate::config::Config>,
    name: &str,
) -> Result<ToolConfig, String> {
    let id = canonical_tool(name);
    let fallback = builtin(id).ok_or_else(|| {
        format!("Unknown tool \"{name}\". Known tools: claude, codex, grok, opencode, pool, pi.")
    })?;
    if let Some(over) = config.and_then(|c| c.tools.get(id)) {
        let mut t = fallback;
        t.merge(over);
        return Ok(t);
    }
    Ok(fallback)
}

pub fn tool_base_url(profile: &Profile, tool: &ToolConfig) -> String {
    format!(
        "{}{}",
        profile.base_url().trim_end_matches('/'),
        tool.base_suffix
    )
}

pub fn default_profile_for_env(base_url: Option<&str>, api_key: Option<&str>) -> Profile {
    Profile {
        api_key: api_key.map(str::to_string),
        base_url: Some(base_url.unwrap_or(DEFAULT_BASE_URL).to_string()),
        pinned_preset: Some(DEFAULT_PRESET.into()),
        default_model: Some("auto".into()),
        timeout_ms: Some(DEFAULT_TIMEOUT_MS),
        ..Profile::default()
    }
}

fn clamp_level(levels: &[&str], level: &str) -> Option<String> {
    if levels.contains(&level) {
        return Some(level.to_string());
    }
    let wanted = REASONING_LEVELS.iter().position(|l| *l == level);
    let mut best = levels.first().copied().unwrap_or(level);
    if let Some(wanted) = wanted {
        for candidate in levels {
            if let Some(rank) = REASONING_LEVELS.iter().position(|l| l == candidate) {
                if rank <= wanted {
                    best = candidate;
                }
            }
        }
    }
    Some(best.to_string())
}

pub fn normalize_effort(input: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = input else {
        return Ok(None);
    };
    let value = raw.trim().to_ascii_lowercase();
    if !REASONING_LEVELS.contains(&value.as_str()) {
        return Err(format!(
            "Invalid --effort \"{raw}\". Expected one of: {}.",
            REASONING_LEVELS.join(", ")
        ));
    }
    Ok(Some(value))
}

fn harness_effort(tool: &str, effort: Option<&str>) -> Option<String> {
    let effort = effort?;
    match tool {
        "claude" => clamp_level(CLAUDE_LEVELS, effort),
        "codex" => clamp_level(CODEX_LEVELS, effort),
        _ => Some(effort.to_string()),
    }
}

pub struct BuildToolEnvInput<'a> {
    pub tool_name: &'a str,
    pub tool: &'a ToolConfig,
    pub profile: &'a Profile,
    pub api_key: &'a str,
    pub model: &'a str,
    pub effort: Option<&'a str>,
    pub context_window: Option<i64>,
    pub model_map: Option<&'a HashMap<String, String>>,
}

pub fn build_tool_env(input: BuildToolEnvInput<'_>) -> BTreeMap<String, String> {
    let model_mode = if input.model.is_empty() || input.model == "auto" {
        "auto"
    } else {
        "concrete"
    };
    let mut env = BTreeMap::new();
    env.insert(
        input.tool.base_url_env.clone(),
        tool_base_url(input.profile, input.tool),
    );
    env.insert(input.tool.auth_env.clone(), input.api_key.to_string());
    env.insert(
        "ANYROUTER_PINNED_PRESET".into(),
        input.profile.pinned_preset().to_string(),
    );
    env.insert("ANYROUTER_MODEL_MODE".into(), model_mode.into());
    env.insert(
        "API_TIMEOUT_MS".into(),
        input.profile.timeout_ms().to_string(),
    );
    if let Some(model_env) = &input.tool.model_env {
        env.insert(model_env.clone(), input.model.to_string());
    }
    if let Some(effort) = input.effort {
        env.insert("ANYROUTER_EFFORT".into(), effort.to_string());
    }
    if input.tool_name == "pi" {
        env.remove(&input.tool.base_url_env);
        let base = tool_base_url(input.profile, input.tool);
        let model_id = if input.model.is_empty() || input.model == "auto" {
            "anthropic/claude-sonnet-4.6"
        } else {
            input.model
        };
        let config = serde_json::json!({
            "providers": {
                "anyrouter": {
                    "baseUrl": base,
                    "api": "openai-completions",
                    "apiKey": "$ANYROUTER_API_KEY",
                    "headers": { "X-AnyRouter-App": "pi" },
                    "models": [{ "id": model_id }]
                }
            }
        });
        env.insert(
            "PI_MODELS_JSON".into(),
            serde_json::to_string(&config).unwrap_or_else(|_| "{}".into()),
        );
    }
    if input.tool_name == "opencode" {
        env.remove(&input.tool.base_url_env);
        if let Some(m) = &input.tool.model_env {
            env.remove(m);
        }
        let mut provider = serde_json::json!({
            "npm": "@ai-sdk/openai-compatible",
            "name": "AnyRouter",
            "options": { "baseURL": tool_base_url(input.profile, input.tool), "apiKey": "{env:OPENAI_API_KEY}" },
            "models": {}
        });
        let mut config = serde_json::json!({
            "$schema": "https://opencode.ai/config.json",
            "provider": { "anyrouter": provider.clone() }
        });
        if !input.model.is_empty() && input.model != "auto" {
            provider["models"][input.model] = serde_json::json!({ "name": input.model });
            config["provider"]["anyrouter"] = provider;
            config["model"] = serde_json::json!(format!("anyrouter/{}", input.model));
        }
        env.insert(
            "OPENCODE_CONFIG_CONTENT".into(),
            serde_json::to_string(&config).unwrap_or_else(|_| "{}".into()),
        );
    }
    if input.tool_name == "claude" {
        env.insert(
            "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY".into(),
            if input.tool.enable_gateway_model_discovery {
                "1"
            } else {
                "0"
            }
            .into(),
        );
        if model_mode == "concrete" && input.model != "auto" {
            for (k, fallback) in [
                ("ANTHROPIC_DEFAULT_HAIKU_MODEL", input.model),
                ("ANTHROPIC_DEFAULT_SONNET_MODEL", input.model),
                ("ANTHROPIC_DEFAULT_OPUS_MODEL", input.model),
                ("CLAUDE_CODE_SUBAGENT_MODEL", input.model),
            ] {
                env.insert(k.into(), fallback.into());
            }
        } else {
            env.insert(
                "ANTHROPIC_DEFAULT_HAIKU_MODEL".into(),
                "anthropic/claude-haiku-4.5".into(),
            );
            env.insert(
                "ANTHROPIC_DEFAULT_SONNET_MODEL".into(),
                "anthropic/claude-sonnet-4.6".into(),
            );
            env.insert(
                "ANTHROPIC_DEFAULT_OPUS_MODEL".into(),
                "anthropic/claude-opus-4.6".into(),
            );
        }
        if let Some(effort) = harness_effort("claude", input.effort) {
            if let Some((_, tokens)) = CLAUDE_EFFORT_TOKENS.iter().find(|(k, _)| *k == effort) {
                env.insert("MAX_THINKING_TOKENS".into(), tokens.to_string());
            }
        }
    }
    let _ = input.context_window;
    let _ = input.model_map;
    env
}

pub fn effort_args_for(tool_name: &str, effort: Option<&str>) -> Vec<String> {
    let Some(mapped) = harness_effort(tool_name, effort) else {
        return vec![];
    };
    if tool_name == "codex" {
        return vec!["-c".into(), format!("model_reasoning_effort=\"{mapped}\"")];
    }
    vec![]
}

pub fn model_args_for(tool_name: &str, model: &str, model_mode: &str) -> Vec<String> {
    if model.is_empty() || model == "auto" || model_mode == "auto" {
        return vec![];
    }
    if tool_name == "pi" {
        return vec!["--model".into(), model.to_string()];
    }
    if tool_name != "codex" {
        return vec![];
    }
    vec!["-c".into(), format!("model=\"{model}\"")]
}

pub fn provider_args_for(tool_name: &str, profile: &Profile) -> Vec<String> {
    if tool_name == "pi" {
        return vec!["--provider".into(), "anyrouter".into()];
    }
    if tool_name != "codex" {
        return vec![];
    }
    let tool = builtin("codex").unwrap();
    let base = tool_base_url(profile, &tool);
    vec![
        "-c".into(),
        "model_provider=\"anyrouter\"".into(),
        "-c".into(),
        "model_providers.anyrouter.name=\"AnyRouter\"".into(),
        "-c".into(),
        format!("model_providers.anyrouter.base_url=\"{base}\""),
        "-c".into(),
        "model_providers.anyrouter.env_key=\"OPENAI_API_KEY\"".into(),
        "-c".into(),
        "model_providers.anyrouter.wire_api=\"responses\"".into(),
        "-c".into(),
        "model_providers.anyrouter.requires_openai_auth=false".into(),
        "-c".into(),
        "model_providers.anyrouter.http_headers={ \"X-AnyRouter-App\" = \"codex\" }".into(),
    ]
}

pub fn redact_value(key: &str, value: &str) -> String {
    let upper = key.to_ascii_uppercase();
    let looks_secret = upper.contains("KEY") || upper.contains("TOKEN") || upper.contains("AUTH");
    if looks_secret && !value.chars().all(|c| c.is_ascii_digit()) {
        if value.len() <= 8 {
            return "<redacted>".into();
        }
        let prefix: String = value.chars().take(6).collect();
        let suffix: String = value.chars().rev().take(4).collect::<String>().chars().rev().collect();
        return format!("{prefix}...{suffix}");
    }
    value.to_string()
}

pub fn render_dry_run(command: &str, args: &[String], env: &BTreeMap<String, String>) -> String {
    let mut lines = vec![
        format!("command: {command}"),
        format!(
            "args: {}",
            serde_json::to_string(args).unwrap_or_else(|_| "[]".into())
        ),
        "env:".into(),
    ];
    for (k, v) in env {
        lines.push(format!("{k}={}", redact_value(k, v)));
    }
    lines.join("\n")
}

pub fn env_command_path(tool: &str, env: &BTreeMap<String, String>) -> Option<String> {
    let key = match canonical_tool(tool) {
        "claude" => "ANYROUTER_CLAUDE_PATH",
        "codex" => "ANYROUTER_CODEX_PATH",
        "grok" => "ANYROUTER_GROK_PATH",
        "opencode" => "ANYROUTER_OPENCODE_PATH",
        "pool" => "ANYROUTER_POOL_PATH",
        "pi" => "ANYROUTER_PI_PATH",
        _ => return None,
    };
    env.get(key)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn spawn_child(command: &str, args: &[String], extra_env: &BTreeMap<String, String>) -> i32 {
    match Command::new(command)
        .args(args)
        .envs(extra_env)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(st) => st.code().unwrap_or(1),
        Err(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        default_profile_for_env(None, Some("sk-ar-v1-secret"))
    }

    #[test]
    fn redact_auth_token_and_keep_numeric() {
        assert_eq!(
            redact_value("ANTHROPIC_AUTH_TOKEN", "sk-ar-v1-secret-value"),
            "sk-ar-...alue"
        );
        assert_eq!(redact_value("MAX_THINKING_TOKENS", "24000"), "24000");
    }

    #[test]
    fn build_tool_env_claude_sets_base_and_auth() {
        let tool = builtin("claude").unwrap();
        let env = build_tool_env(BuildToolEnvInput {
            tool_name: "claude",
            tool: &tool,
            profile: &profile(),
            api_key: "sk-ar-v1-secret",
            model: "auto",
            effort: None,
            context_window: None,
            model_map: None,
        });
        assert_eq!(
            env.get("ANTHROPIC_BASE_URL").map(String::as_str),
            Some("https://anyrouter.dev/api")
        );
        assert_eq!(
            env.get("ANTHROPIC_AUTH_TOKEN").map(String::as_str),
            Some("sk-ar-v1-secret")
        );
    }

    #[test]
    fn build_tool_env_codex_base_ends_with_v1() {
        let tool = builtin("codex").unwrap();
        let env = build_tool_env(BuildToolEnvInput {
            tool_name: "codex",
            tool: &tool,
            profile: &profile(),
            api_key: "sk-ar-v1-secret",
            model: "x",
            effort: None,
            context_window: None,
            model_map: None,
        });
        assert!(env.get("OPENAI_BASE_URL").unwrap().ends_with("/v1"));
    }

    #[test]
    fn build_tool_env_grok_has_base_no_model_env() {
        let tool = builtin("grok").unwrap();
        let env = build_tool_env(BuildToolEnvInput {
            tool_name: "grok",
            tool: &tool,
            profile: &profile(),
            api_key: "sk-ar-v1-secret",
            model: "x",
            effort: None,
            context_window: None,
            model_map: None,
        });
        assert!(env.contains_key("GROK_MODELS_BASE_URL"));
        assert!(tool.model_env.is_none());
    }

    #[test]
    fn render_dry_run_contains_base_and_redacts_key() {
        let tool = builtin("claude").unwrap();
        let env = build_tool_env(BuildToolEnvInput {
            tool_name: "claude",
            tool: &tool,
            profile: &profile(),
            api_key: "sk-ar-v1-secret-value",
            model: "auto",
            effort: None,
            context_window: None,
            model_map: None,
        });
        let out = render_dry_run("claude", &[], &env);
        assert!(out.contains("ANTHROPIC_BASE_URL"));
        assert!(!out.contains("sk-ar-v1-secret-value"));
    }

    #[test]
    fn build_tool_env_pi_sets_models_json_and_auth() {
        let tool = builtin("pi").unwrap();
        let env = build_tool_env(BuildToolEnvInput {
            tool_name: "pi",
            tool: &tool,
            profile: &profile(),
            api_key: "sk-ar-v1-secret",
            model: "z-ai/glm-4.7-flash",
            effort: None,
            context_window: None,
            model_map: None,
        });
        assert_eq!(
            env.get("ANYROUTER_API_KEY").map(String::as_str),
            Some("sk-ar-v1-secret")
        );
        let json = env.get("PI_MODELS_JSON").expect("PI_MODELS_JSON");
        assert!(json.contains("anyrouter.dev/api/v1"), "{json}");
        assert!(json.contains("z-ai/glm-4.7-flash"), "{json}");
        assert!(json.contains("$ANYROUTER_API_KEY"), "{json}");
        assert!(!json.contains("sk-ar-v1-secret"), "{json}");
        assert_eq!(
            provider_args_for("pi", &profile()),
            vec!["--provider".to_string(), "anyrouter".to_string()]
        );
        assert_eq!(
            model_args_for("pi", "z-ai/glm-4.7-flash", "concrete"),
            vec!["--model".to_string(), "z-ai/glm-4.7-flash".to_string()]
        );
    }

    #[test]
    fn claude_minimal_effort_clamps_to_low() {
        let tool = builtin("claude").unwrap();
        let env = build_tool_env(BuildToolEnvInput {
            tool_name: "claude",
            tool: &tool,
            profile: &profile(),
            api_key: "sk-ar-v1-secret",
            model: "anthropic/claude-sonnet-4.6",
            effort: Some("minimal"),
            context_window: None,
            model_map: None,
        });
        assert_eq!(
            env.get("MAX_THINKING_TOKENS").map(String::as_str),
            Some("2048")
        );
    }
}
