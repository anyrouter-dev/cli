use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
#[cfg(feature = "native")]
use std::process::{Command, Stdio};

use crate::config::{
    Profile, YamlValue, DEFAULT_BASE_URL, DEFAULT_MODEL, DEFAULT_PRESET, DEFAULT_TIMEOUT_MS,
};

pub const PI_DEFAULT_MODEL: &str = "anthropic/claude-sonnet-4.6";
/// Historical 1M-context suffix used by Claude Code. Claude Code used to strip
/// it before the gateway, but third-party catalog ids (e.g.
/// `meituan/longcat-2.0`) are forwarded verbatim to the provider and 404 on
/// `id[1m]`. The CLI no longer appends it; we keep stripping it on ingest as
/// defense against a user-typed or relay-tagged id.
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

#[derive(Debug, Clone, PartialEq, Default)]
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
        default_model: Some(DEFAULT_MODEL.into()),
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
    /// Peeled `[1m]`/`[500k]` or `routing.min_context`. Not catalog `context_length`.
    pub min_context: Option<i64>,
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
            model_id_for_tool(input.tool_name, input.model, input.min_context),
        );
    }
    if let Some(effort) = input.effort {
        env.insert("ANYROUTER_EFFORT".into(), effort.to_string());
    }
    if input.tool_name == "pi" {
        // Placeholder until `prepare_pi_wrapper` overwrites with the full catalog.
        let base = tool_base_url(input.profile, input.tool);
        let model_id = pi_resolved_model(input.model);
        let ids = vec![model_id];
        env.insert(
            "PI_MODELS_JSON".into(),
            serde_json::to_string(&pi_models_config(&base, &ids)).unwrap_or_else(|_| "{}".into()),
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
        let anthropic_model = model_id_for_tool("claude", input.model, input.min_context);
        env.insert("ANTHROPIC_MODEL".into(), anthropic_model);
        env.insert(
            "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY".into(),
            // Discovery remaps unknown ids (including virtual `anyrouter/*`)
            // onto a catalog SKU such as Laguna. Keep it off for presets so the
            // gateway still sees the virtual id + extra-body min_context.
            if claude_gateway_discovery_enabled(input.tool, input.model) {
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
        let pinned = (!is_virtual_preset(input.model)).then_some(input.model);
        let alias = |slot: &Option<String>, default: &str| -> String {
            let explicit = slot.as_deref().map(str::trim).filter(|s| !s.is_empty());
            match (pinned, explicit) {
                (Some(id), None) => model_id_for_tool("claude", id, input.min_context),
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
        if claude_wants_auto_compact(input.model, input.min_context, input.context_window) {
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

/// First-party virtual presets (`anyrouter/auto`, `anyrouter/free`, …).
/// Keep in lockstep with gateway `ANYROUTER_VIRTUAL_MODELS`.
const VIRTUAL_PRESETS: &[&str] = &[
    "anyrouter/auto",
    "anyrouter/free",
    "anyrouter/byok",
    "anyrouter/coding",
    "anyrouter/agent",
    "anyrouter/hermes",
    "anyrouter/cowork",
    "anyrouter/latest",
];

pub fn is_auto_model(model: &str) -> bool {
    let value = catalog_model_id(model);
    let value = crate::config::strip_context_window_suffix(&value);
    value.is_empty() || value == "auto" || value == "anyrouter/auto"
}

/// Virtual routing presets. `[1m]` / `[500k]` are min_context floors, not listing ids.
pub fn is_virtual_preset(model: &str) -> bool {
    if is_auto_model(model) {
        return true;
    }
    let id = catalog_model_id(model);
    let id = crate::config::strip_context_window_suffix(&id);
    VIRTUAL_PRESETS.iter().any(|p| *p == id)
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
        // Unknown concrete window: still enable compact so a 1M session is not
        // truncated at Claude's 200k default. Virtual presets use the floor.
        None => true,
    }
}

fn merged_floor(model: &str, min_context: Option<i64>) -> Option<i64> {
    match (peel_context_window_suffixes(model).1, min_context) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

/// Compact when the routing floor is ≥ 1M. Concrete ids without a floor still
/// use catalog `context_length` (unknown counts as yes).
pub fn claude_wants_auto_compact(
    model: &str,
    min_context: Option<i64>,
    context_window: Option<i64>,
) -> bool {
    if merged_floor(model, min_context).is_some_and(|n| n >= MIN_1M_CONTEXT) {
        return true;
    }
    if is_virtual_preset(model) {
        return false;
    }
    claude_wants_1m(context_window)
}

pub fn claude_gateway_discovery_enabled(tool: &ToolConfig, model: &str) -> bool {
    tool.enable_gateway_model_discovery && !is_virtual_preset(model)
}

/// Catalog `context_length` for a concrete id. Virtual `anyrouter/*` must not
/// inherit auto's 200k listing — that would paint the HUD `[200k]`.
pub fn catalog_context_window(
    requested: &str,
    models: &[crate::http::CatalogModel],
) -> Option<i64> {
    let id = catalog_model_id(requested);
    if is_virtual_preset(&id) {
        return None;
    }
    models
        .iter()
        .find(|m| catalog_model_id(&m.id) == id)
        .and_then(|m| m.context_length)
}

/// Spell a token floor as `[1m]` / `[500k]` (same as `--model` suffixes).
pub fn context_floor_suffix(n: i64) -> Option<String> {
    if n <= 0 {
        return None;
    }
    if n % 1_000_000 == 0 {
        return Some(format!("[{}m]", n / 1_000_000));
    }
    if n % 1_000 == 0 {
        return Some(format!("[{}k]", n / 1_000));
    }
    None
}

/// Peel `[Nm]`/`[Nk]` → catalog id. Claude virtual presets re-attach the floor
/// so the HUD shows `[1m]`/`[500k]` instead of catalog 200k. Concrete ids never
/// get an invented suffix (those SKUs 404).
pub fn model_id_for_tool(tool_name: &str, model: &str, min_context: Option<i64>) -> String {
    let (peeled, peeled_floor) = peel_context_window_suffixes(model);
    let id = catalog_model_id(&peeled);
    let catalog = if is_auto_model(&id) {
        display_model_id(&id)
    } else {
        id
    };
    if tool_name == "claude" && is_virtual_preset(&catalog) {
        if let Some(n) = match (peeled_floor, min_context) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        } {
            if let Some(sfx) = context_floor_suffix(n) {
                return format!("{catalog}{sfx}");
            }
        }
    }
    catalog
}

/// Strip CSI, Claude's `[1m]` 1M suffix, closed `[Nm]` / `[0;1m]` tails, and
/// dangling SGR (`[1m` without `]`). Store/send the catalog id only.
pub fn sanitize_model_id(model: &str) -> String {
    catalog_model_id(model)
}

/// Parse trailing `[<n>k|m]` as a token floor (`k` = thousand, `m` = million).
/// Same spelling as `anyrouter/auto[1m]` / `[500k]`. Does not strip CSI.
pub fn peel_context_window_suffixes(model: &str) -> (String, Option<i64>) {
    let mut s = model.trim().to_string();
    let mut min_context: Option<i64> = None;
    loop {
        let Some(open) = s.rfind('[') else {
            break;
        };
        if !s.ends_with(']') || open + 2 >= s.len() {
            break;
        }
        let inner = &s[open + 1..s.len() - 1];
        let Some(unit) = inner.chars().last() else {
            break;
        };
        let digits = &inner[..inner.len() - 1];
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            break;
        }
        let n: i64 = match digits.parse() {
            Ok(n) if n > 0 => n,
            _ => break,
        };
        let floor = match unit {
            'm' | 'M' => n.saturating_mul(1_000_000),
            'k' | 'K' => n.saturating_mul(1_000),
            _ => break,
        };
        min_context = Some(match min_context {
            Some(existing) => existing.max(floor),
            None => floor,
        });
        s.truncate(open);
    }
    (s, min_context)
}

/// Peel `[1m]` / `[500k]` (and `:exacto`) into routing prefs; return catalog id.
pub fn apply_model_id_routing(
    model: &str,
    routing: &mut crate::config::RoutingConstraints,
) -> String {
    let (mut peeled, floor) = peel_context_window_suffixes(model);
    if let Some(n) = floor {
        routing.merge_min_context(n);
    }
    if let Some((base, suffix)) = peeled.rsplit_once(':') {
        if suffix.eq_ignore_ascii_case(crate::config::ROUTING_SORT_EXACTO) {
            routing.set_exacto(true);
            let (base2, floor2) = peel_context_window_suffixes(base);
            if let Some(n) = floor2 {
                routing.merge_min_context(n);
            }
            peeled = base2;
        }
    }
    catalog_model_id(&peeled)
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
    // Trailing `[1m]` / `[500k]` floors, CSI-like `[0;1m]`, dangling `[1m`.
    if crate::config::parse_context_window_suffix(&s).is_some() {
        s = crate::config::strip_context_window_suffix(&s).to_string();
    }
    if let Some(i) = s.rfind('[') {
        let tail = &s[i + 1..];
        let codes = tail.strip_suffix(']').unwrap_or(tail);
        if codes.ends_with('m')
            && codes.len() > 1
            && codes[..codes.len() - 1]
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

/// Build Pi `models.json` for the AnyRouter provider.
///
/// Pi's `/model` picker only lists ids present under `providers.anyrouter.models`.
/// Writing a single selected id made the picker show one row (e.g.
/// `dots-studio/dots-3-note-preview`) with "Only showing models from configured
/// providers". Pass the full catalog (selected first) so `/model` can switch.
pub fn pi_models_config(base_url: &str, model_ids: &[String]) -> serde_json::Value {
    let models: Vec<serde_json::Value> = model_ids
        .iter()
        .filter(|id| !id.is_empty())
        .map(|id| serde_json::json!({ "id": id }))
        .collect();
    serde_json::json!({
        "providers": {
            "anyrouter": {
                "baseUrl": base_url,
                "api": "openai-completions",
                "apiKey": "ANYROUTER_API_KEY",
                "authHeader": true,
                "headers": { "X-AnyRouter-App": "pi" },
                "models": models
            }
        }
    })
}

/// Deduped catalog ids for Pi, with `selected` first (after `pi_resolved_model`).
pub fn pi_catalog_model_ids(selected: &str, catalog_ids: &[String]) -> Vec<String> {
    let selected = pi_resolved_model(selected);
    let mut out = Vec::with_capacity(catalog_ids.len().saturating_add(1));
    if !selected.is_empty() {
        out.push(selected.clone());
    }
    for id in catalog_ids {
        let id = catalog_model_id(id);
        if id.is_empty() || id == selected || out.iter().any(|x| x == &id) {
            continue;
        }
        out.push(id);
    }
    if out.is_empty() && !selected.is_empty() {
        out.push(selected);
    }
    out
}

pub fn pi_agent_dir(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .map(|p| p.join("pi"))
        .unwrap_or_else(|| PathBuf::from("pi"))
}

/// Pi reads `models.json` from `PI_CODING_AGENT_DIR` (not `PI_MODELS_JSON`).
/// Write a wrap dir with AnyRouter already registered and the full catalog listed.
///
/// `catalog_ids` should be live `/v1/models` ids (any order). The selected
/// `model` is listed first; if `catalog_ids` is empty, only the selected id is
/// written (offline / fetch-failure fallback).
pub fn prepare_pi_wrapper(
    env: &mut BTreeMap<String, String>,
    config_path: &Path,
    profile: &Profile,
    tool: &ToolConfig,
    model: &str,
    catalog_ids: &[String],
) -> Result<(), String> {
    let dir = pi_agent_dir(config_path);
    let model_id = pi_resolved_model(model);
    let base = tool_base_url(profile, tool);
    let ids = pi_catalog_model_ids(model, catalog_ids);
    let models = pi_models_config(&base, &ids);
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
        assert_eq!(
            sanitize_model_id("stealth/ox-alpha[2m]"),
            "stealth/ox-alpha"
        );
        assert_eq!(
            sanitize_model_id("stealth/ox-alpha[0;1m]"),
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
    fn model_id_for_tool_never_appends_1m_for_claude() {
        assert_eq!(
            model_id_for_tool("claude", "stealth/ox-alpha", None),
            "stealth/ox-alpha"
        );
        assert_eq!(
            model_id_for_tool("claude", "stealth/ox-alpha[1m]", None),
            "stealth/ox-alpha"
        );
        assert_eq!(
            model_id_for_tool("claude", "stealth/ox-alpha", Some(200_000)),
            "stealth/ox-alpha"
        );
        assert_eq!(
            model_id_for_tool("claude", "stealth/ox-alpha", Some(1_000_000)),
            "stealth/ox-alpha"
        );
        assert_eq!(
            model_id_for_tool("claude", "stealth/ox-alpha[2m]", None),
            "stealth/ox-alpha"
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
        assert_eq!(
            model_id_for_tool("claude", "anyrouter/auto[1m]", None),
            "anyrouter/auto[1m]"
        );
        assert_eq!(
            model_id_for_tool("claude", "anyrouter/auto[500k]", None),
            "anyrouter/auto[500k]"
        );
        assert_eq!(
            model_id_for_tool("claude", "anyrouter/auto", Some(1_000_000)),
            "anyrouter/auto[1m]"
        );
        assert_eq!(
            model_id_for_tool("claude", "anyrouter/free[1m]", None),
            "anyrouter/free[1m]"
        );
        assert_eq!(
            model_id_for_tool("pi", "anyrouter/auto[1m]", None),
            "anyrouter/auto"
        );
        assert!(is_auto_model("anyrouter/auto[1m]"));
        assert!(is_auto_model("anyrouter/auto[500k]"));
        assert_eq!(catalog_model_id("anyrouter/auto[500k]"), "anyrouter/auto");
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
            min_context: None,
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
            min_context: None,
            model_map: None,
        });
        assert_eq!(
            env.get("ANTHROPIC_MODEL").map(String::as_str),
            Some("stealth/ox-alpha")
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
                Some("stealth/ox-alpha"),
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
            min_context: None,
            model_map: None,
        });
        assert_eq!(
            env.get("ANTHROPIC_DEFAULT_SONNET_MODEL")
                .map(String::as_str),
            Some("z-ai/glm-4.7-flash")
        );
        for key in [
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "CLAUDE_CODE_SUBAGENT_MODEL",
        ] {
            assert_eq!(
                env.get(key).map(String::as_str),
                Some("stealth/ox-alpha"),
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
            min_context: None,
            model_map: None,
        });
        assert_eq!(
            env.get("ANTHROPIC_MODEL").map(String::as_str),
            Some("anyrouter/auto")
        );
        assert_eq!(
            env.get("ANTHROPIC_DEFAULT_HAIKU_MODEL").map(String::as_str),
            Some("z-ai/glm-4.7-flash")
        );
        assert_eq!(
            env.get("CLAUDE_CODE_SUBAGENT_MODEL").map(String::as_str),
            Some("z-ai/glm-4.7-flash")
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
            min_context: None,
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
            min_context: None,
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
            min_context: None,
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
            min_context: None,
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
        let catalog = vec![
            "anyrouter/free".to_string(),
            "anthropic/claude-sonnet-4.6".to_string(),
            "z-ai/glm-5.2".to_string(),
        ];
        prepare_pi_wrapper(
            &mut env,
            &cfg,
            &profile(),
            &tool,
            "anyrouter/free",
            &catalog,
        )
        .unwrap();
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
        assert!(models.contains("anthropic/claude-sonnet-4.6"), "{models}");
        assert!(models.contains("z-ai/glm-5.2"), "{models}");
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
    fn pi_catalog_model_ids_puts_selected_first_and_dedupes() {
        let ids = pi_catalog_model_ids(
            "z-ai/glm-5.2",
            &[
                "anyrouter/free".into(),
                "z-ai/glm-5.2".into(),
                "anyrouter/free".into(),
            ],
        );
        assert_eq!(
            ids,
            vec!["z-ai/glm-5.2".to_string(), "anyrouter/free".to_string(),]
        );
    }

    #[test]
    fn pi_models_config_lists_every_id() {
        let json = pi_models_config(
            "https://anyrouter.dev/api/v1",
            &["a/b".into(), "c/d".into()],
        );
        let models = json["providers"]["anyrouter"]["models"]
            .as_array()
            .expect("models array");
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["id"], "a/b");
        assert_eq!(models[1]["id"], "c/d");
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
            min_context: None,
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
        assert!(!env.contains_key("CLAUDE_CODE_EXTRA_BODY"));
        routing.set_exacto(true);
        routing.set_require_tools(true);
        routing.set_require_1m(true);
        apply_routing_env(&mut env, &routing, "claude");
        let body = env.get("CLAUDE_CODE_EXTRA_BODY").expect("extra body");
        assert!(body.contains("\"sort\":\"exacto\""), "{body}");
        assert!(body.contains("\"require_params\":[\"tools\"]"), "{body}");
        assert!(body.contains("\"min_context\":1000000"), "{body}");
        assert!(body.contains("\"provider\""), "{body}");
        assert_eq!(env.get("ANYROUTER_EXTRA_BODY"), Some(body));
    }

    #[test]
    fn peel_auto_1m_and_500k_into_min_context() {
        let (id, floor) = peel_context_window_suffixes("anyrouter/auto[1m]");
        assert_eq!(id, "anyrouter/auto");
        assert_eq!(floor, Some(1_000_000));
        let (id, floor) = peel_context_window_suffixes("anyrouter/auto[500k]");
        assert_eq!(id, "anyrouter/auto");
        assert_eq!(floor, Some(500_000));
        let mut routing = crate::config::RoutingConstraints::default();
        let catalog = apply_model_id_routing("anyrouter/auto[1m]:exacto", &mut routing);
        assert_eq!(catalog, "anyrouter/auto");
        assert!(is_auto_model(&catalog));
        assert_eq!(routing.min_context, Some(1_000_000));
        assert!(routing.wants_exacto());
        let body = routing.extra_body_json().expect("body");
        assert!(body.contains("\"min_context\":1000000"), "{body}");
        assert!(body.contains("\"sort\":\"exacto\""), "{body}");
        for id in [
            "anyrouter/free[1m]",
            "anyrouter/byok[1m]",
            "anyrouter/hermes[500k]",
            "anyrouter/latest[1m]",
        ] {
            let mut r = crate::config::RoutingConstraints::default();
            let catalog = apply_model_id_routing(id, &mut r);
            assert!(is_virtual_preset(id), "{id}");
            assert!(!catalog.contains('['), "{catalog}");
            assert!(r.min_context.is_some(), "{id}");
        }
    }

    #[test]
    fn claude_virtual_auto_1m_sets_anthropic_model_suffix_and_compact() {
        let tool = builtin("claude").unwrap();
        let mut env = build_tool_env(BuildToolEnvInput {
            tool_name: "claude",
            tool: &tool,
            profile: &profile(),
            api_key: "sk-ar-v1-secret",
            model: "anyrouter/auto[1m]",
            effort: None,
            context_window: None,
            min_context: None,
            model_map: None,
        });
        let mut routing = crate::config::RoutingConstraints::default();
        let catalog = apply_model_id_routing("anyrouter/auto[1m]", &mut routing);
        assert_eq!(catalog, "anyrouter/auto");
        apply_routing_env(&mut env, &routing, "claude");
        assert_eq!(
            env.get("ANTHROPIC_MODEL").map(String::as_str),
            Some("anyrouter/auto[1m]")
        );
        let body = env.get("CLAUDE_CODE_EXTRA_BODY").expect("extra body");
        assert!(body.contains("\"min_context\":1000000"), "{body}");
        assert_eq!(
            env.get("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
                .map(String::as_str),
            Some("1000000")
        );

        // Launch peels the suffix first and only passes routing.min_context.
        let peeled = build_tool_env(BuildToolEnvInput {
            tool_name: "claude",
            tool: &tool,
            profile: &profile(),
            api_key: "sk-ar-v1-secret",
            model: "anyrouter/auto",
            effort: None,
            context_window: Some(200_000),
            min_context: Some(1_000_000),
            model_map: None,
        });
        assert_eq!(
            peeled.get("ANTHROPIC_MODEL").map(String::as_str),
            Some("anyrouter/auto[1m]"),
            "do not use catalog 200k as the HUD suffix"
        );
        assert_eq!(
            peeled
                .get("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
                .map(String::as_str),
            Some("1000000")
        );
        assert!(!claude_gateway_discovery_enabled(
            &tool,
            "anyrouter/auto[1m]"
        ));
        let auto_row = crate::http::CatalogModel {
            id: "anyrouter/auto".into(),
            name: None,
            owned_by: None,
            context_length: Some(200_000),
        };
        let ox = crate::http::CatalogModel {
            id: "stealth/ox-alpha".into(),
            name: None,
            owned_by: None,
            context_length: Some(1_000_000),
        };
        assert_eq!(
            catalog_context_window("anyrouter/auto[1m]", &[auto_row.clone(), ox.clone()]),
            None,
            "virtual preset must not inherit catalog 200k"
        );
        assert_eq!(
            catalog_context_window("stealth/ox-alpha", &[auto_row, ox]),
            Some(1_000_000)
        );
        assert!(claude_wants_auto_compact(
            "anyrouter/auto",
            Some(1_000_000),
            Some(200_000)
        ));
        assert!(!claude_wants_auto_compact(
            "anyrouter/auto[500k]",
            None,
            Some(200_000)
        ));

        let half = build_tool_env(BuildToolEnvInput {
            tool_name: "claude",
            tool: &tool,
            profile: &profile(),
            api_key: "sk-ar-v1-secret",
            model: "anyrouter/auto[500k]",
            effort: None,
            context_window: None,
            min_context: None,
            model_map: None,
        });
        assert_eq!(
            half.get("ANTHROPIC_MODEL").map(String::as_str),
            Some("anyrouter/auto[500k]")
        );
        assert!(half.get("CLAUDE_CODE_AUTO_COMPACT_WINDOW").is_none());
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
            min_context: None,
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
        assert_eq!(
            env.get("CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY")
                .map(String::as_str),
            Some("0"),
            "auto must not be remapped by catalog discovery"
        );
        let free = build_tool_env(BuildToolEnvInput {
            tool_name: "claude",
            tool: &tool,
            profile: &profile(),
            api_key: "sk-ar-v1-secret",
            model: "anyrouter/free[1m]",
            effort: None,
            context_window: None,
            min_context: None,
            model_map: None,
        });
        assert_eq!(
            free.get("ANTHROPIC_MODEL").map(String::as_str),
            Some("anyrouter/free[1m]")
        );
        assert_eq!(
            free.get("CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY")
                .map(String::as_str),
            Some("0")
        );
        let concrete = build_tool_env(BuildToolEnvInput {
            tool_name: "claude",
            tool: &tool,
            profile: &profile(),
            api_key: "sk-ar-v1-secret",
            model: "poolside/laguna-s-2.1",
            effort: None,
            context_window: None,
            min_context: None,
            model_map: None,
        });
        assert_eq!(
            concrete
                .get("CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY")
                .map(String::as_str),
            Some("1")
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
        let over = ToolConfig {
            command: "/opt/codex".into(),
            ..Default::default()
        };
        t.merge(&over);
        assert_eq!(t.command, "/opt/codex");
        assert_eq!(t.base_suffix, "/v1");
    }
}
