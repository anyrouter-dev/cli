use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

pub static VALUE_FLAGS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "model",
        "effort",
        "preset",
        "key",
        "profile",
        "base-url",
        "config",
        "command-path",
        "claude-path",
        "timeout",
        "system",
        "plan-model",
        "do-model",
        "to",
        "from",
        "limit",
        "status",
        "type",
        "hub",
        "source",
        "tool",
        "agent",
        "channel",
        "fixture",
        "haiku",
        "sonnet",
        "opus",
        "fable",
        "target",
        "token",
        "url",
        "name",
        "max-concurrency",
    ])
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlagValue {
    Bool(bool),
    Value(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedArgs {
    pub command: String,
    pub flags: HashMap<String, FlagValue>,
    pub passthrough: Vec<String>,
}

impl ParsedArgs {
    pub fn flag_true(&self, name: &str) -> bool {
        matches!(self.flags.get(name), Some(FlagValue::Bool(true)))
    }

    /// `--yes` or `--ok` skip confirmation prompts (login / install / revoke).
    pub fn skip_confirm(&self) -> bool {
        self.flag_true("yes") || self.flag_true("ok")
    }
}

pub fn parse_cli_args<I, S>(argv: I) -> Result<ParsedArgs, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let tokens: Vec<String> = argv.into_iter().map(|s| s.as_ref().to_string()).collect();
    let command = tokens
        .first()
        .cloned()
        .unwrap_or_else(|| "help".to_string());
    let rest = if tokens.is_empty() {
        &[][..]
    } else {
        &tokens[1..]
    };

    let mut flags = HashMap::new();
    let mut passthrough = Vec::new();
    let mut after_separator = false;
    let mut i = 0usize;

    while i < rest.len() {
        let arg = &rest[i];
        if after_separator {
            passthrough.push(arg.clone());
            i += 1;
            continue;
        }
        if arg == "--" {
            after_separator = true;
            i += 1;
            continue;
        }
        if !arg.starts_with("--") {
            passthrough.push(arg.clone());
            i += 1;
            continue;
        }
        if let Some(eq) = arg.find('=') {
            let eq_name = arg[2..eq].to_string();
            let eq_value = arg[eq + 1..].to_string();
            if VALUE_FLAGS.contains(eq_name.as_str()) {
                if eq_value.is_empty() {
                    return Err(format!("Flag --{eq_name} requires a value."));
                }
                flags.insert(eq_name, FlagValue::Value(eq_value));
            } else {
                flags.insert(
                    eq_name.clone(),
                    FlagValue::Bool(parse_bool_flag(&eq_name, &eq_value)?),
                );
            }
            i += 1;
            continue;
        }
        let name = arg[2..].to_string();
        if VALUE_FLAGS.contains(name.as_str()) {
            let next = rest.get(i + 1);
            if next.is_none()
                || next == Some(&"--".to_string())
                || next.is_some_and(|n| n.starts_with("--"))
            {
                return Err(format!("Flag --{name} requires a value."));
            }
            flags.insert(name, FlagValue::Value(rest[i + 1].clone()));
            i += 2;
        } else {
            flags.insert(name, FlagValue::Bool(true));
            i += 1;
        }
    }

    Ok(ParsedArgs {
        command,
        flags,
        passthrough,
    })
}

fn parse_bool_flag(name: &str, raw: &str) -> Result<bool, String> {
    let t = raw.trim();
    if t.is_empty()
        || t == "1"
        || t.eq_ignore_ascii_case("true")
        || t.eq_ignore_ascii_case("yes")
        || t.eq_ignore_ascii_case("on")
    {
        return Ok(true);
    }
    if t == "0"
        || t.eq_ignore_ascii_case("false")
        || t.eq_ignore_ascii_case("no")
        || t.eq_ignore_ascii_case("off")
    {
        return Ok(false);
    }
    Err(format!("Flag --{name} does not take a value ({raw})."))
}

pub fn get_string_flag(flags: &HashMap<String, FlagValue>, name: &str) -> Option<String> {
    match flags.get(name) {
        Some(FlagValue::Value(value)) if !value.is_empty() => Some(value.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_yes_is_boolean_true() {
        let parsed = parse_cli_args(["login", "--yes"]).unwrap();
        assert_eq!(parsed.command, "login");
        assert!(parsed.flag_true("yes"));
    }

    #[test]
    fn model_without_value_errors() {
        assert!(parse_cli_args(["claude", "--model"])
            .unwrap_err()
            .contains("--model requires a value"));
        assert!(parse_cli_args(["claude", "--model="])
            .unwrap_err()
            .contains("--model requires a value"));
    }

    #[test]
    fn account_list_goes_to_passthrough() {
        let parsed = parse_cli_args(["account", "list"]).unwrap();
        assert_eq!(parsed.passthrough, vec!["list"]);
    }

    #[test]
    fn auth_login_goes_to_passthrough() {
        let parsed = parse_cli_args(["auth", "login", "--yes"]).unwrap();
        assert_eq!(parsed.command, "auth");
        assert_eq!(parsed.passthrough, vec!["login"]);
        assert!(parsed.flag_true("yes"));
    }

    #[test]
    fn separator_starts_passthrough() {
        let parsed = parse_cli_args(["claude", "--yes", "--", "--print"]).unwrap();
        assert_eq!(parsed.passthrough, vec!["--print"]);
    }

    #[test]
    fn bool_flag_equals_form_is_still_true() {
        // WHY: `--yes=true` used to land as FlagValue::Value, so flag_true missed it.
        let parsed = parse_cli_args(["login", "--yes=true"]).unwrap();
        assert!(parsed.flag_true("yes"));
        assert!(parsed.skip_confirm());
        let device = parse_cli_args(["login", "--device=1"]).unwrap();
        assert!(device.flag_true("device"));
        let off = parse_cli_args(["login", "--yes=false"]).unwrap();
        assert!(!off.flag_true("yes"));
    }

    #[test]
    fn ok_is_confirm_alias() {
        let parsed = parse_cli_args(["claude", "--ok"]).unwrap();
        assert!(parsed.skip_confirm());
        assert!(parsed.flag_true("ok"));
    }
}
