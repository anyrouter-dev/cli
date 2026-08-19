use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::spawn::{create_default_tools, ToolConfig};

pub const DEFAULT_BASE_URL: &str = "https://anyrouter.dev/api";
pub const DEFAULT_TIMEOUT_MS: i64 = 3_000_000;
pub const DEFAULT_PROFILE: &str = "default";
pub const DEFAULT_PRESET: &str = "@preset/coding-stack";

#[derive(Debug, Clone, PartialEq)]
pub enum YamlValue {
    Map(BTreeMap<String, YamlValue>),
    String(String),
    Bool(bool),
    Int(i64),
    Null,
}

impl YamlValue {
    pub fn as_map(&self) -> Option<&BTreeMap<String, YamlValue>> {
        match self {
            YamlValue::Map(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            YamlValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_string_lossy(&self) -> String {
        match self {
            YamlValue::String(s) => s.clone(),
            YamlValue::Bool(b) => b.to_string(),
            YamlValue::Int(n) => n.to_string(),
            YamlValue::Null => String::new(),
            YamlValue::Map(_) => String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Profile {
    pub api_key: Option<String>,
    pub management_key: Option<String>,
    pub base_url: Option<String>,
    pub pinned_preset: Option<String>,
    pub default_model: Option<String>,
    pub timeout_ms: Option<i64>,
    pub extra: BTreeMap<String, YamlValue>,
}

impl Profile {
    pub fn base_url(&self) -> &str {
        self.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL)
    }

    pub fn pinned_preset(&self) -> &str {
        self.pinned_preset.as_deref().unwrap_or(DEFAULT_PRESET)
    }

    pub fn default_model(&self) -> &str {
        self.default_model.as_deref().unwrap_or("auto")
    }

    pub fn timeout_ms(&self) -> i64 {
        self.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub active_profile: String,
    pub last_tool: Option<String>,
    pub profiles: BTreeMap<String, Profile>,
    pub tools: BTreeMap<String, ToolConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            active_profile: DEFAULT_PROFILE.into(),
            last_tool: None,
            profiles: BTreeMap::new(),
            tools: create_default_tools(),
        }
    }
}

pub struct DefaultProfileInput {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub preset: Option<String>,
    pub timeout_ms: Option<i64>,
    pub default_model: Option<String>,
}

pub fn normalize_preset(input: &str) -> String {
    let value = input.trim();
    if let Some(rest) = value.strip_prefix("@presets/") {
        return format!("@preset/{rest}");
    }
    if value.starts_with("@preset/") {
        return value.to_string();
    }
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return format!("@preset/{value}");
    }
    value.to_string()
}

pub fn create_default_profile(input: DefaultProfileInput) -> Profile {
    Profile {
        api_key: input.api_key,
        base_url: Some(input.base_url.unwrap_or_else(|| DEFAULT_BASE_URL.into())),
        pinned_preset: Some(normalize_preset(
            input.preset.as_deref().unwrap_or(DEFAULT_PRESET),
        )),
        default_model: Some(input.default_model.unwrap_or_else(|| "auto".into())),
        timeout_ms: Some(input.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS)),
        ..Profile::default()
    }
}

pub fn resolve_config_path(flag: Option<&str>, env: &BTreeMap<String, String>) -> PathBuf {
    if let Some(p) = flag.filter(|s| !s.is_empty()) {
        return PathBuf::from(p);
    }
    if let Some(home) = env
        .get("ANYROUTER_HOME")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        return PathBuf::from(home).join("config.yaml");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".anyrouter")
        .join("config.yaml")
}

pub fn get_active_profile<'a>(
    config: &'a Config,
    profile_name: Option<&str>,
    env: &BTreeMap<String, String>,
) -> Result<&'a Profile, String> {
    let env_name = env
        .get("ANYROUTER_PROFILE")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let name = profile_name
        .filter(|s| !s.is_empty())
        .or(env_name)
        .unwrap_or(config.active_profile.as_str());
    config
        .profiles
        .get(name)
        .ok_or_else(|| format!("Profile \"{name}\" was not found in AnyRouter config."))
}

pub fn upsert_profile(mut config: Config, name: &str, profile: Profile) -> Config {
    config.profiles.insert(name.to_string(), profile);
    if config.active_profile.is_empty() {
        config.active_profile = name.to_string();
    }
    config
}

fn parse_yaml_scalar(raw: &str) -> YamlValue {
    match raw {
        "true" => YamlValue::Bool(true),
        "false" => YamlValue::Bool(false),
        "null" => YamlValue::Null,
        _ if raw.chars().all(|c| c.is_ascii_digit() || c == '-') && raw.parse::<i64>().is_ok() => {
            YamlValue::Int(raw.parse().unwrap())
        }
        _ if raw.starts_with('"') => serde_json::from_str::<String>(raw)
            .map(YamlValue::String)
            .unwrap_or_else(|_| YamlValue::String(raw.trim_matches('"').into())),
        _ => YamlValue::String(raw.to_string()),
    }
}

fn yaml_scalar(value: &str) -> String {
    if value.is_empty()
        || value.contains([':', '#', '@'])
        || value.starts_with(' ')
        || value.ends_with(' ')
    {
        return serde_json::to_string(value).unwrap_or_else(|_| format!("\"{value}\""));
    }
    value.to_string()
}

pub fn yaml_scalar_value(value: &YamlValue) -> String {
    match value {
        YamlValue::Bool(b) => b.to_string(),
        YamlValue::Int(n) => n.to_string(),
        YamlValue::Null => "null".into(),
        YamlValue::String(s) => yaml_scalar(s),
        YamlValue::Map(_) => String::new(),
    }
}

fn map_at_path<'a>(
    root: &'a mut BTreeMap<String, YamlValue>,
    path: &[String],
) -> &'a mut BTreeMap<String, YamlValue> {
    let mut cur = root;
    for key in path {
        cur = match cur.get_mut(key) {
            Some(YamlValue::Map(m)) => m,
            _ => panic!("yaml path missing {key}"),
        };
    }
    cur
}

fn parse_yaml_map(source: &str) -> BTreeMap<String, YamlValue> {
    let mut root = BTreeMap::new();
    let mut stack: Vec<(isize, Vec<String>)> = vec![(-1, vec![])];

    for raw_line in source.split('\n') {
        let raw_line = raw_line.trim_end_matches('\r');
        if raw_line.trim().is_empty() || raw_line.trim().starts_with('#') {
            continue;
        }
        let indent = raw_line.len() - raw_line.trim_start().len();
        let line = raw_line.trim();
        let Some(colon) = line.find(':') else {
            continue;
        };
        let key = line[..colon].trim().to_string();
        let raw_value = line[colon + 1..].trim();

        while stack.len() > 1 && indent as isize <= stack.last().unwrap().0 {
            stack.pop();
        }
        let parent_path = stack.last().unwrap().1.clone();
        let parent = map_at_path(&mut root, &parent_path);

        if raw_value.is_empty() {
            parent.insert(key.clone(), YamlValue::Map(BTreeMap::new()));
            let mut child_path = parent_path;
            child_path.push(key);
            stack.push((indent as isize, child_path));
        } else {
            parent.insert(key, parse_yaml_scalar(raw_value));
        }
    }
    root
}

fn profile_from_map(map: &BTreeMap<String, YamlValue>) -> Profile {
    let mut p = Profile::default();
    for (k, v) in map {
        match k.as_str() {
            "api_key" => p.api_key = Some(v.as_string_lossy()).filter(|s| !s.is_empty()),
            "management_key" => p.management_key = Some(v.as_string_lossy()).filter(|s| !s.is_empty()),
            "base_url" => p.base_url = Some(v.as_string_lossy()),
            "pinned_preset" => p.pinned_preset = Some(normalize_preset(&v.as_string_lossy())),
            "default_model" => p.default_model = Some(v.as_string_lossy()),
            "timeout_ms" => {
                p.timeout_ms = match v {
                    YamlValue::Int(n) => Some(*n),
                    _ => v.as_string_lossy().parse().ok(),
                }
            }
            _ => {
                p.extra.insert(k.clone(), v.clone());
            }
        }
    }
    p
}

pub fn parse_config(source: &str) -> Config {
    let root = parse_yaml_map(source);
    let mut config = Config::default();
    if let Some(s) = root.get("active_profile").and_then(|v| v.as_str()) {
        config.active_profile = s.to_string();
    }
    if let Some(s) = root.get("last_tool").and_then(|v| v.as_str()) {
        config.last_tool = Some(s.to_string());
    }
    if let Some(map) = root.get("profiles").and_then(|v| v.as_map()) {
        for (name, value) in map {
            if let Some(pm) = value.as_map() {
                config.profiles.insert(name.clone(), profile_from_map(pm));
            }
        }
    }
    if let Some(map) = root.get("tools").and_then(|v| v.as_map()) {
        for (name, value) in map {
            if let Some(tm) = value.as_map() {
                let mut tool = config
                    .tools
                    .get(name)
                    .cloned()
                    .unwrap_or_else(ToolConfig::default);
                tool.merge(&ToolConfig::from_yaml(tm));
                config.tools.insert(name.clone(), tool);
            }
        }
    }
    config
}

pub fn serialize_config(config: &Config) -> String {
    let mut lines = vec![format!(
        "active_profile: {}",
        yaml_scalar(&config.active_profile)
    )];
    lines.push("profiles:".into());
    for (name, profile) in &config.profiles {
        lines.push(format!("  {name}:"));
        if let Some(k) = &profile.api_key {
            lines.push(format!("    api_key: {}", yaml_scalar(k)));
        }
        if let Some(u) = &profile.base_url {
            lines.push(format!("    base_url: {}", yaml_scalar(u)));
        }
        if let Some(p) = &profile.pinned_preset {
            lines.push(format!("    pinned_preset: {}", yaml_scalar(p)));
        }
        if let Some(m) = &profile.default_model {
            lines.push(format!("    default_model: {}", yaml_scalar(m)));
        }
        if let Some(t) = profile.timeout_ms {
            lines.push(format!("    timeout_ms: {t}"));
        }
    }
    lines.push("tools:".into());
    for (name, tool) in &config.tools {
        lines.push(format!("  {name}:"));
        lines.extend(tool.to_yaml_lines());
    }
    format!("{}\n", lines.join("\n"))
}

pub fn read_config(path: &Path) -> Result<Config, String> {
    let src = fs::read_to_string(path).map_err(|_| {
        "Missing AnyRouter config. Run: npx @anyr/cli claude (first run sets you up).".to_string()
    })?;
    Ok(parse_config(&src))
}

pub fn write_config(config: &Config, path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Could not create config dir: {e}"))?;
    }
    fs::write(path, serialize_config(config)).map_err(|e| format!("Could not write config: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honors_anyrouter_home() {
        let mut env = BTreeMap::new();
        env.insert("ANYROUTER_HOME".into(), "/tmp/xyz".into());
        assert_eq!(
            resolve_config_path(None, &env),
            PathBuf::from("/tmp/xyz/config.yaml")
        );
    }

    #[test]
    fn parse_config_roundtrip_minimal_yaml() {
        let src = "\
active_profile: default
profiles:
  default:
    api_key: sk-ar-v1-test
    base_url: https://anyrouter.dev/api
    pinned_preset: \"@preset/coding-stack\"
    default_model: auto
    timeout_ms: 3000000
";
        let cfg = parse_config(src);
        assert_eq!(
            cfg.profiles.get("default").unwrap().api_key.as_deref(),
            Some("sk-ar-v1-test")
        );
        let again = parse_config(&serialize_config(&cfg));
        assert_eq!(
            again.profiles.get("default").unwrap().api_key.as_deref(),
            Some("sk-ar-v1-test")
        );
    }
}
