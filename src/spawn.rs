use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
#[cfg(feature = "native")]
use std::process::{Command, Stdio};

use crate::config::{Profile, YamlValue, DEFAULT_BASE_URL, DEFAULT_PRESET, DEFAULT_TIMEOUT_MS};

pub const PI_DEFAULT_MODEL: &str = "anthropic/claude-sonnet-4.6";
/// Claude Code 1M-context suffix. Claude strips it before the gateway; other
/// agents must never send it (Pi/Codex 404 on `id[1m]`).
pub const CLAUDE_1M_SUFFIX: &str = "[1m]";
const MIN_1M_CONTEXT: i64 = 1_000_000;

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
        if !over.base_suffix.is_empty() {
            self.base_suffix = over.base_suffix.clone();
        }
        // Overlay from `from_yaml` defaults discovery to false when the key
        // is missing — never copy that over a builtin. `apply_yaml` is the
        // path that honors an explicit false.
        if over.enable_gateway_model_discovery {
            self.enable_gateway_model_discovery = true;
        }
        if over.shadow_env.is_some() {
            self.shadow_env = over.shadow_env.clone();
        }
        for (k, v) in &over.extra {
            self.extra.insert(k.clone(), v.clone());
        }
    }

    /// Apply only keys present in `map`. Missing keys keep the current value
    /// so a partial `tools.claude.command:` overlay cannot wipe `/v1` or
    /// gateway discovery.
    pub fn apply_yaml(&mut self, map: &BTreeMap<String, YamlValue>) {
        for (key, value) in map {
            match key.as_str() {
                "command" => self.command = value.as_string_lossy(),
                "base_url_env" => self.base_url_env = value.as_string_lossy(),
                "auth_env" => self.auth_env = value.as_string_lossy(),
                "model_env" => {
                    let s = value.as_string_lossy();
                    self.model_env = if s.is_empty() || s == "null" {
                        None
                    } else {
                        Some(s)
                    };
                }
                "base_suffix" => self.base_suffix = value.as_string_lossy(),
                "enable_gateway_model_discovery" => {
                    self.enable_gateway_model_discovery =
                        matches!(value, YamlValue::Bool(true)) || value.as_string_lossy() == "true"
                }
                "shadow_env" => {
                    let s = value.as_string_lossy();
                    self.shadow_env = if s.is_empty() || s == "null" {
                        None
                    } else {
                        Some(s)
                    };
                }
                _ => {
                    self.extra.insert(key.clone(), value.clone());
                }
            }
        }
    }

    pub fn from_yaml(map: &BTreeMap<String, YamlValue>) -> Self {
        let mut tool = ToolConfig::default();
        tool.apply_yaml(map);
        tool
    }

    pub fn extra_flag(&self, key: &str) -> bool {
        match self.extra.get(key) {
            Some(YamlValue::Bool(true)) => true,
            Some(YamlValue::Int(n)) => *n != 0,
            Some(YamlValue::String(s)) => {
                let t = s.trim();
                t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
            }
            _ => false,
        }
    }

    pub fn to_yaml_lines(&self) -> Vec<String> {
        let mut lines = vec![
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
        ];
        for (key, value) in &self.extra {
            lines.push(format!(
                "    {key}: {}",
                crate::config::yaml_scalar_value(value)
            ));
        }
        lines
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
        // Parsed tools are already builtin + apply_yaml. Clone, don't merge a
        // second time (merge would treat missing overlay keys as defaults).
        return Ok(over.clone());
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
    let model_mode = if is_auto_model(input.model) {
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
    // Parent-shell Anthropic/OpenAI keys must not beat the AnyRouter token.
    if let Some(shadow) = &input.tool.shadow_env {
        env.insert(shadow.clone(), input.api_key.to_string());
    }
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
        env.insert(
            model_env.clone(),
            model_id_for_tool(input.tool_name, input.model, input.context_window),
        );
    }
    if let Some(effort) = input.effort {
        env.insert("ANYROUTER_EFFORT".into(), effort.to_string());
    }
    if input.tool_name == "pi" {
        let base = tool_base_url(input.profile, input.tool);
        let model_id = pi_resolved_model(input.model);
        env.insert(
            "PI_MODELS_JSON".into(),
            serde_json::to_string(&pi_models_config(&base, &model_id))
                .unwrap_or_else(|_| "{}".into()),
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
        let catalog = catalog_model_id(input.model);
        if !catalog.is_empty() && !is_auto_model(&catalog) {
            provider["models"][&catalog] = serde_json::json!({ "name": catalog });
            config["provider"]["anyrouter"] = provider;
            config["model"] = serde_json::json!(format!("anyrouter/{catalog}"));
        }
        env.insert(
            "OPENCODE_CONFIG_CONTENT".into(),
            serde_json::to_string(&config).unwrap_or_else(|_| "{}".into()),
        );
    }
    if input.tool_name == "claude" {
        env.insert(
            "ANTHROPIC_MODEL".into(),
            model_id_for_tool("claude", input.model, input.context_window),
        );
        env.insert(
            "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY".into(),
            if input.tool.enable_gateway_model_discovery {
                "1"
            } else {
                "0"
            }
            .into(),
        );
        // A concrete pinned model takes over every Claude Code alias slot
        // (haiku / sonnet / opus / fable and subagents) so nothing — including
        // automatic model fallback, which rides the fable alias on third-party
        // providers — falls back to a different model. Slots set explicitly
        // (--haiku/--sonnet/--opus/--fable or the profile config) still win.
        let pinned = (!is_auto_model(input.model)).then_some(input.model);
        let alias = |slot: &Option<String>, default: &str| -> String {
            let explicit = slot.as_deref().map(str::trim).filter(|s| !s.is_empty());
            match (pinned, explicit) {
                (Some(id), None) => model_id_for_tool("claude", id, input.context_window),
                (None, Some(id)) => model_id_for_tool("claude", id, None),
                (Some(_), Some(id)) => model_id_for_tool("claude", id, None),
                (None, None) => default.to_string(),
            }
        };
        let haiku = alias(&input.profile.claude_haiku, input.profile.claude_haiku());
        env.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL".into(), haiku.clone());
        env.insert(
            "ANTHROPIC_DEFAULT_SONNET_MODEL".into(),
            alias(&input.profile.claude_sonnet, input.profile.claude_sonnet()),
        );
        env.insert(
            "ANTHROPIC_DEFAULT_OPUS_MODEL".into(),
            alias(&input.profile.claude_opus, input.profile.claude_opus()),
        );
        env.insert(
            "ANTHROPIC_DEFAULT_FABLE_MODEL".into(),
            alias(&input.profile.claude_fable, input.profile.claude_fable()),
        );
        env.insert("CLAUDE_CODE_SUBAGENT_MODEL".into(), haiku);
        if !is_auto_model(input.model) && claude_wants_1m(input.context_window) {
            env.insert("CLAUDE_CODE_AUTO_COMPACT_WINDOW".into(), "1000000".into());
        }
        // Label each picker entry with its role; otherwise four identical IDs
        // all render as "Custom <Alias> model".
        for (key, value) in [
            (
                "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION",
                "Background & subagents",
            ),
            ("ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION", "Sonnet alias"),
            ("ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION", "Opus alias"),
            (
                "ANTHROPIC_DEFAULT_FABLE_MODEL_DESCRIPTION",
                "Fable alias + fallback",
            ),
        ] {
            env.insert(key.into(), value.into());
        }
        if let Some(effort) = harness_effort("claude", input.effort) {
            if let Some((_, tokens)) = CLAUDE_EFFORT_TOKENS.iter().find(|(k, _)| *k == effort) {
                env.insert("MAX_THINKING_TOKENS".into(), tokens.to_string());
            }
        }
    }
    let _ = input.model_map;
    env
}

/// Merge AnyRouter preset routing fields into the child env so launch
/// actually sends them. Claude Code reads `CLAUDE_CODE_EXTRA_BODY`.
pub fn apply_routing_env(
    env: &mut BTreeMap<String, String>,
    routing: &crate::config::RoutingConstraints,
    tool_name: &str,
) {
    let Some(body) = routing.extra_body_json() else {
        return;
    };
    env.insert("ANYROUTER_EXTRA_BODY".into(), body.clone());
    if tool_name == "claude" {
        env.insert("CLAUDE_CODE_EXTRA_BODY".into(), body);
    }
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

pub fn is_auto_model(model: &str) -> bool {
    let value = catalog_model_id(model);
    value.is_empty() || value == "auto" || value == "anyrouter/auto"
}

/// Catalog id for display and config. Auto is `anyrouter/auto`.
pub fn display_model_id(model: &str) -> String {
    let id = catalog_model_id(model);
    if is_auto_model(&id) {
        "anyrouter/auto".into()
    } else {
        id
    }
}

/// Launcher / settings label. Auto is the selectable preset `anyrouter/auto`.
pub fn session_model_label(model: &str) -> String {
    display_model_id(model)
}

pub fn claude_wants_1m(context_window: Option<i64>) -> bool {
    match context_window {
        Some(n) => n >= MIN_1M_CONTEXT,
        // Unknown: Claude Code strips `[1m]` before the provider, so appending
        // is safe for the gateway and unlocks 1M when the model supports it.
        None => true,
    }
}

/// Agent-specific model id. Claude Code gets `[1m]` (1M context). Pi/Codex/etc
/// get the catalog id — the suffix 404s on the OpenAI-compatible API.
pub fn model_id_for_tool(tool_name: &str, model: &str, context_window: Option<i64>) -> String {
    let id = catalog_model_id(model);
    if is_auto_model(&id) || id.starts_with("anyrouter/") {
        return if is_auto_model(&id) {
            display_model_id(&id)
        } else {
            id
        };
    }
    if tool_name == "claude" && claude_wants_1m(context_window) {
        if id.ends_with(CLAUDE_1M_SUFFIX) {
            id
        } else {
            format!("{id}{CLAUDE_1M_SUFFIX}")
        }
    } else {
        id
    }
}

/// Strip CSI, Claude's `[1m]` 1M suffix, and dangling SGR tails (`[1m` without
/// `]`). Config and non-Claude agents store/send the catalog id only.
pub fn sanitize_model_id(model: &str) -> String {
    catalog_model_id(model)
}

pub fn catalog_model_id(model: &str) -> String {
    let mut s = String::with_capacity(model.len());
    let mut chars = model.trim().chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        s.push(c);
    }
    if let Some(stripped) = s.strip_suffix(CLAUDE_1M_SUFFIX) {
        s = stripped.to_string();
    }
    if let Some(i) = s.rfind('[') {
        let tail = &s[i + 1..];
        if tail.ends_with('m')
            && tail.len() > 1
            && tail[..tail.len() - 1]
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b';')
        {
            s.truncate(i);
        }
    }
    s.trim().to_string()
}

pub fn pi_resolved_model(model: &str) -> String {
    let s = catalog_model_id(model);
    if is_auto_model(&s) {
        PI_DEFAULT_MODEL.to_string()
    } else {
        s
    }
}

pub fn pi_models_config(base_url: &str, model_id: &str) -> serde_json::Value {
    serde_json::json!({
        "providers": {
            "anyrouter": {
                "baseUrl": base_url,
                "api": "openai-completions",
                "apiKey": "ANYROUTER_API_KEY",
                "authHeader": true,
                "headers": { "X-AnyRouter-App": "pi" },
                "models": [{ "id": model_id }]
            }
        }
    })
}

pub fn pi_agent_dir(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .map(|p| p.join("pi"))
        .unwrap_or_else(|| PathBuf::from("pi"))
}

/// Pi reads `models.json` from `PI_CODING_AGENT_DIR` (not `PI_MODELS_JSON`).
/// Write a wrap dir with AnyRouter already registered and selected.
pub fn prepare_pi_wrapper(
    env: &mut BTreeMap<String, String>,
    config_path: &Path,
    profile: &Profile,
    tool: &ToolConfig,
    model: &str,
) -> Result<(), String> {
    let dir = pi_agent_dir(config_path);
    let model_id = pi_resolved_model(model);
    let base = tool_base_url(profile, tool);
    let models = pi_models_config(&base, &model_id);
    write_pi_wrapper_files(&dir, &models, &model_id)?;
    env.insert(
        "PI_CODING_AGENT_DIR".into(),
        dir.to_string_lossy().into_owned(),
    );
    env.insert(
        "PI_MODELS_JSON".into(),
        serde_json::to_string(&models).unwrap_or_else(|_| "{}".into()),
    );
    Ok(())
}

#[cfg(feature = "native")]
fn write_pi_wrapper_files(
    dir: &Path,
    models: &serde_json::Value,
    model_id: &str,
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| {
        format!(
            "Could not create Pi wrapper directory {}: {e}",
            dir.display()
        )
    })?;
    std::fs::write(
        dir.join("models.json"),
        serde_json::to_vec_pretty(models).unwrap_or_else(|_| b"{}".to_vec()),
    )
    .map_err(|e| format!("Could not write Pi models.json: {e}"))?;
    let settings = serde_json::json!({
        "defaultProvider": "anyrouter",
        "defaultModel": model_id,
    });
    std::fs::write(
        dir.join("settings.json"),
        serde_json::to_vec_pretty(&settings).unwrap_or_else(|_| b"{}".to_vec()),
    )
    .map_err(|e| format!("Could not write Pi settings.json: {e}"))?;
    Ok(())
}

#[cfg(not(feature = "native"))]
fn write_pi_wrapper_files(
    dir: &Path,
    models: &serde_json::Value,
    model_id: &str,
) -> Result<(), String> {
    let _ = (dir, models, model_id);
    Ok(())
}

pub fn model_args_for(tool_name: &str, model: &str, model_mode: &str) -> Vec<String> {
    if tool_name == "pi" {
        let id = pi_resolved_model(model);
        // Official AnyRouter Pi guide: `pi --provider anyrouter --model "<catalog-id>"`.
        // The provider is already `--provider anyrouter`; do not prefix it again
        // (`anyrouter/stealth/ox-alpha` 404s — the catalog id is `stealth/ox-alpha`).
        return vec!["--model".into(), id];
    }
    let catalog = catalog_model_id(model);
    if catalog.is_empty() || is_auto_model(&catalog) || model_mode == "auto" {
        return vec![];
    }
    if tool_name != "codex" {
        return vec![];
    }
    vec!["-c".into(), format!("model=\"{catalog}\"")]
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
        let suffix: String = value
            .chars()
            .rev()
            .take(4)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
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

#[cfg(not(feature = "native"))]
pub fn spawn_child(command: &str, args: &[String], extra_env: &BTreeMap<String, String>) -> i32 {
    let _ = (args, extra_env);
    eprintln!("spawn is not available in the browser demo ({command})");
    1
}

#[cfg(feature = "native")]
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
        Err(err) => {
            eprintln!("Could not start \"{command}\": {err}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        default_profile_for_env(None, Some("sk-ar-v1-secret"))
    }

    #[test]
    fn sanitize_model_id_strips_ansi_and_claude_1m_suffix() {
        assert_eq!(sanitize_model_id("stealth/ox-alpha"), "stealth/ox-alpha");
        assert_eq!(
            sanitize_model_id("stealth/ox-alpha\u{1b}[1m"),
            "stealth/ox-alpha"
        );
        assert_eq!(sanitize_model_id("stealth/ox-alpha[1m"), "stealth/ox-alpha");
        assert_eq!(
            sanitize_model_id("stealth/ox-alpha[1m]"),
            "stealth/ox-alpha"
        );
        assert_eq!(
            sanitize_model_id("\u{1b}[1mstealth/ox-alpha\u{1b}[0m"),
            "stealth/ox-alpha"
        );
        assert_eq!(pi_resolved_model("anyrouter/auto"), PI_DEFAULT_MODEL);
        assert_eq!(pi_resolved_model("auto"), PI_DEFAULT_MODEL);
        assert_eq!(
            pi_resolved_model("stealth/ox-alpha[1m]"),
            "stealth/ox-alpha"
        );
    }

    #[test]
    fn model_id_for_tool_appends_1m_only_for_claude() {
        assert_eq!(
            model_id_for_tool("claude", "stealth/ox-alpha", None),
            "stealth/ox-alpha[1m]"
        );
        assert_eq!(
            model_id_for_tool("claude", "stealth/ox-alpha[1m]", None),
            "stealth/ox-alpha[1m]"
        );
        assert_eq!(
            model_id_for_tool("claude", "stealth/ox-alpha", Some(200_000)),
            "stealth/ox-alpha"
        );
        assert_eq!(
            model_id_for_tool("claude", "stealth/ox-alpha", Some(1_000_000)),
            "stealth/ox-alpha[1m]"
        );
        assert_eq!(
            model_id_for_tool("pi", "stealth/ox-alpha[1m]", None),
            "stealth/ox-alpha"
        );
        assert_eq!(
            model_id_for_tool("codex", "stealth/ox-alpha[1m]", Some(1_000_000)),
            "stealth/ox-alpha"
        );
        assert_eq!(model_id_for_tool("claude", "auto", None), "anyrouter/auto");
        assert_eq!(
            model_id_for_tool("claude", "anyrouter/auto", None),
            "anyrouter/auto"
        );
        assert_eq!(
            model_id_for_tool("claude", "anyrouter/free", None),
            "anyrouter/free"
        );
    }

    #[test]
    fn session_model_label_is_anyrouter_auto_preset() {
        assert_eq!(session_model_label("auto"), "anyrouter/auto");
        assert_eq!(session_model_label("anyrouter/auto"), "anyrouter/auto");
        assert_eq!(session_model_label(""), "anyrouter/auto");
        assert_eq!(
            session_model_label("stealth/ox-alpha[1m]"),
            "stealth/ox-alpha"
        );
        assert_ne!(session_model_label("auto"), "auto  ·  most used");
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
        assert_eq!(
            env.get("ANTHROPIC_MODEL").map(String::as_str),
            Some("anyrouter/auto")
        );
        assert_eq!(
            env.get("ANTHROPIC_DEFAULT_HAIKU_MODEL").map(String::as_str),
            Some("anthropic/claude-haiku-4.5")
        );
        assert_eq!(
            env.get("ANTHROPIC_DEFAULT_SONNET_MODEL")
                .map(String::as_str),
            Some("anthropic/claude-sonnet-4.6")
        );
        assert_eq!(
            env.get("ANTHROPIC_DEFAULT_OPUS_MODEL").map(String::as_str),
            Some("anthropic/claude-opus-4.6")
        );
    }

    #[test]
    fn claude_pinned_model_collapses_unset_aliases() {
        let tool = builtin("claude").unwrap();
        let env = build_tool_env(BuildToolEnvInput {
            tool_name: "claude",
            tool: &tool,
            profile: &profile(),
            api_key: "sk-ar-v1-secret",
            model: "stealth/ox-alpha",
            effort: None,
            context_window: None,
            model_map: None,
        });
        assert_eq!(
            env.get("ANTHROPIC_MODEL").map(String::as_str),
            Some("stealth/ox-alpha[1m]")
        );
        assert_eq!(
            env.get("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
                .map(String::as_str),
            Some("1000000")
        );
        // Every unset alias slot follows the pinned model so nothing
        // (subagents, automatic fallback) silently falls back to another model.
        for key in [
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "CLAUDE_CODE_SUBAGENT_MODEL",
        ] {
            assert_eq!(
                env.get(key).map(String::as_str),
                Some("stealth/ox-alpha[1m]"),
                "{key} should follow the pinned model"
            );
        }
    }

    #[test]
    fn claude_explicit_alias_beats_pinned_model() {
        let tool = builtin("claude").unwrap();
        let mut p = profile();
        p.claude_sonnet = Some("z-ai/glm-4.7-flash".into());
        let env = build_tool_env(BuildToolEnvInput {
            tool_name: "claude",
            tool: &tool,
            profile: &p,
            api_key: "sk-ar-v1-secret",
            model: "stealth/ox-alpha",
            effort: None,
            context_window: None,
            model_map: None,
        });
        assert_eq!(
            env.get("ANTHROPIC_DEFAULT_SONNET_MODEL")
                .map(String::as_str),
            Some("z-ai/glm-4.7-flash[1m]")
        );
        for key in [
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "CLAUDE_CODE_SUBAGENT_MODEL",
        ] {
            assert_eq!(
                env.get(key).map(String::as_str),
                Some("stealth/ox-alpha[1m]"),
                "{key} should follow the pinned model"
            );
        }
    }

    #[test]
    fn claude_uses_profile_alias_overrides() {
        let tool = builtin("claude").unwrap();
        let mut p = profile();
        p.claude_haiku = Some("z-ai/glm-4.7-flash".into());
        p.claude_sonnet = Some("anthropic/claude-sonnet-4.6".into());
        p.claude_opus = Some("anthropic/claude-opus-4.6".into());
        let env = build_tool_env(BuildToolEnvInput {
            tool_name: "claude",
            tool: &tool,
            profile: &p,
            api_key: "sk-ar-v1-secret",
            model: "anyrouter/auto",
            effort: None,
            context_window: None,
            model_map: None,
        });
        assert_eq!(
            env.get("ANTHROPIC_MODEL").map(String::as_str),
            Some("anyrouter/auto")
        );
        assert_eq!(
            env.get("ANTHROPIC_DEFAULT_HAIKU_MODEL").map(String::as_str),
            Some("z-ai/glm-4.7-flash[1m]")
        );
        assert_eq!(
            env.get("CLAUDE_CODE_SUBAGENT_MODEL").map(String::as_str),
            Some("z-ai/glm-4.7-flash[1m]")
        );
        assert_eq!(
            env.get("ANYROUTER_MODEL_MODE").map(String::as_str),
            Some("auto")
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
        assert!(json.contains("\"apiKey\":\"ANYROUTER_API_KEY\""), "{json}");
        assert!(!json.contains("$ANYROUTER_API_KEY"), "{json}");
        assert!(!json.contains("sk-ar-v1-secret"), "{json}");
        assert_eq!(
            provider_args_for("pi", &profile()),
            vec!["--provider".to_string(), "anyrouter".to_string()]
        );
        assert_eq!(
            model_args_for("pi", "z-ai/glm-4.7-flash", "concrete"),
            vec!["--model".to_string(), "z-ai/glm-4.7-flash".to_string()]
        );
        assert_eq!(
            model_args_for("pi", "anyrouter/free", "concrete"),
            vec!["--model".to_string(), "anyrouter/free".to_string()]
        );
        assert_eq!(
            model_args_for("pi", "stealth/ox-alpha", "concrete"),
            vec!["--model".to_string(), "stealth/ox-alpha".to_string()]
        );
        assert_eq!(
            model_args_for("pi", "stealth/ox-alpha[1m", "concrete"),
            vec!["--model".to_string(), "stealth/ox-alpha".to_string()]
        );
        assert_eq!(
            model_args_for("pi", "stealth/ox-alpha[1m]", "concrete"),
            vec!["--model".to_string(), "stealth/ox-alpha".to_string()]
        );
        assert_eq!(
            model_args_for("pi", "auto", "auto"),
            vec!["--model".to_string(), PI_DEFAULT_MODEL.to_string()]
        );
        assert_eq!(
            model_args_for("pi", "anyrouter/auto", "auto"),
            vec!["--model".to_string(), PI_DEFAULT_MODEL.to_string()]
        );
    }

    #[test]
    fn prepare_pi_wrapper_writes_models_json() {
        let dir = std::env::temp_dir().join(format!("anyr-pi-wrap-{}", std::process::id()));
        let cfg = dir.join("config.yaml");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tool = builtin("pi").unwrap();
        let mut env = BTreeMap::new();
        prepare_pi_wrapper(&mut env, &cfg, &profile(), &tool, "anyrouter/free").unwrap();
        let agent = dir.join("pi");
        let agent_s = agent.to_string_lossy().into_owned();
        assert_eq!(
            env.get("PI_CODING_AGENT_DIR").map(String::as_str),
            Some(agent_s.as_str())
        );
        let models = std::fs::read_to_string(agent.join("models.json")).unwrap();
        assert!(models.contains("ANYROUTER_API_KEY"), "{models}");
        assert!(!models.contains("$ANYROUTER_API_KEY"), "{models}");
        assert!(models.contains("anyrouter/free"), "{models}");
        assert!(models.contains("anyrouter.dev/api/v1"), "{models}");
        let settings = std::fs::read_to_string(agent.join("settings.json")).unwrap();
        assert!(
            settings.contains("\"defaultProvider\": \"anyrouter\""),
            "{settings}"
        );
        assert!(settings.contains("anyrouter/free"), "{settings}");
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn apply_routing_env_sets_claude_extra_body() {
        let mut env = BTreeMap::new();
        let mut routing = crate::config::RoutingConstraints::default();
        apply_routing_env(&mut env, &routing, "claude");
        assert!(env.get("CLAUDE_CODE_EXTRA_BODY").is_none());
        routing.set_exacto(true);
        routing.set_require_tools(true);
        routing.set_require_1m(true);
        apply_routing_env(&mut env, &routing, "claude");
        let body = env.get("CLAUDE_CODE_EXTRA_BODY").expect("extra body");
        assert!(body.contains("\"sort\":\"exacto\""), "{body}");
        assert!(body.contains("\"require_params\":[\"tools\"]"), "{body}");
        assert!(body.contains("\"min_context\":1000000"), "{body}");
        assert_eq!(env.get("ANYROUTER_EXTRA_BODY"), Some(body));
    }

    #[test]
    fn claude_shadow_env_overrides_parent_anthropic_key() {
        // WHY: a leftover ANTHROPIC_API_KEY in the parent shell must not win.
        let tool = builtin("claude").unwrap();
        assert_eq!(tool.shadow_env.as_deref(), Some("ANTHROPIC_API_KEY"));
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
            env.get("ANTHROPIC_AUTH_TOKEN").map(String::as_str),
            Some("sk-ar-v1-secret")
        );
        assert_eq!(
            env.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("sk-ar-v1-secret")
        );
    }

    #[test]
    fn apply_yaml_partial_does_not_wipe_gateway_discovery() {
        let mut tool = builtin("claude").unwrap();
        let mut map = BTreeMap::new();
        map.insert("command".into(), YamlValue::String("/opt/claude".into()));
        tool.apply_yaml(&map);
        assert_eq!(tool.command, "/opt/claude");
        assert!(
            tool.enable_gateway_model_discovery,
            "partial YAML must keep builtin discovery"
        );
    }

    #[test]
    fn extra_yolo_round_trips_in_yaml() {
        let mut tool = builtin("claude").unwrap();
        let mut map = BTreeMap::new();
        map.insert("yolo".into(), YamlValue::Bool(true));
        tool.apply_yaml(&map);
        assert!(tool.extra_flag("yolo"));
        let yaml = tool.to_yaml_lines().join("\n");
        assert!(yaml.contains("yolo: true"), "{yaml}");
        let parsed = crate::config::parse_config(
            "active_profile: default\nprofiles:\n  default:\n    api_key: x\ntools:\n  claude:\n    yolo: true\n",
        );
        let again = parsed.tools.get("claude").cloned().unwrap();
        assert!(
            again.extra_flag("yolo"),
            "parse_config must keep extra yolo"
        );
    }

    #[test]
    fn merge_command_only_overlay_keeps_codex_suffix() {
        let mut t = builtin("codex").unwrap();
        let mut over = ToolConfig::default();
        over.command = "/opt/codex".into();
        t.merge(&over);
        assert_eq!(t.command, "/opt/codex");
        assert_eq!(t.base_suffix, "/v1");
    }
}
