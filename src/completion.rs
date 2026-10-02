//! Shell completion. One engine (`anyr __complete <words…>`) answers every
//! shell; the scripts from `anyr completion <shell>` are thin shims that call
//! it, so the command grammar lives only in Rust (cobra's model).
//!
//! Protocol: one candidate per line, `value` or `value\tdescription`.
//! Never prints to stderr, never prompts, never colors.

use std::collections::BTreeMap;

use crate::cmd::dispatch::{allowed_flags, value_choices, COMMANDS};

pub const SHELLS: &[&str] = &["bash", "zsh", "fish", "powershell"];

/// Subcommands per command, shared with help/dispatch tests.
pub fn subcommands(command: &str) -> &'static [(&'static str, &'static str)] {
    match command {
        "auth" => &[
            ("login", "Sign in"),
            ("logout", "Remove the stored key"),
            ("status", "Show the signed-in account"),
            ("token", "Print the active API key"),
            ("switch", "Switch the active account"),
        ],
        "account" => &[
            ("list", "List accounts"),
            ("use", "Switch the active account"),
            ("add", "Sign in to another account"),
        ],
        "keys" => &[
            ("list", "List API keys"),
            ("create", "Create an API key"),
            ("use", "Store a key on this machine"),
            ("revoke", "Revoke an API key"),
        ],
        "config" => &[
            ("get", "Print current settings"),
            ("path", "Print the config file path"),
            ("use", "Switch the active account"),
        ],
        "completion" => &[
            ("bash", "Bash completion script"),
            ("zsh", "Zsh completion script"),
            ("fish", "Fish completion script"),
            ("powershell", "PowerShell completion script"),
        ],
        "models" => &[
            ("list", "List catalog model ids"),
            ("ls", "Alias of list"),
            ("use", "Set the default model"),
        ],
        "byok" => &[
            ("list", "List provider keys with BYOK / DONATED badges"),
            (
                "add",
                "Add a provider key (--donate shares it with the pool)",
            ),
        ],
        "api" => crate::api::RESOURCES,
        _ => &[],
    }
}

/// Candidates for the word being completed. `words` excludes the binary name;
/// the last element is the (possibly empty) partial word.
pub fn complete(words: &[String], env: &BTreeMap<String, String>) -> Vec<String> {
    let (current, before) = match words.split_last() {
        Some((cur, rest)) => (cur.as_str(), rest),
        None => ("", &[][..]),
    };
    let prefixed = |items: Vec<(String, String)>| -> Vec<String> {
        items
            .into_iter()
            .filter(|(v, _)| v.starts_with(current))
            .map(|(v, d)| if d.is_empty() { v } else { format!("{v}\t{d}") })
            .collect()
    };

    // After `--` everything belongs to the wrapped agent.
    if before.iter().any(|w| w == "--") {
        return Vec::new();
    }

    let Some(command) = before.first().map(String::as_str) else {
        return prefixed(
            COMMANDS
                .iter()
                .map(|(n, d)| (n.to_string(), d.to_string()))
                .collect(),
        );
    };

    // Value for the previous `--flag`.
    if let Some(flag) = before.last().and_then(|w| w.strip_prefix("--")) {
        if before.len() > 1 && crate::parse::VALUE_FLAGS.contains(flag) {
            return prefixed(
                value_choices(flag, env)
                    .into_iter()
                    .map(|v| (v, String::new()))
                    .collect(),
            );
        }
    }

    if current.starts_with('-') {
        let canonical = crate::cmd::dispatch::canonical_name(command);
        let mut flags: Vec<(String, String)> = allowed_flags(canonical)
            .unwrap_or(&[])
            .iter()
            .map(|f| (format!("--{f}"), String::new()))
            .collect();
        flags.push(("--help".into(), String::new()));
        return prefixed(flags);
    }

    let positional: Vec<&str> = before[1..]
        .iter()
        .map(String::as_str)
        .filter(|w| !w.starts_with('-'))
        .collect();
    let canonical = crate::cmd::dispatch::canonical_name(command);
    match (canonical, positional.as_slice()) {
        ("api", [resource]) => prefixed(
            crate::api::verbs(resource)
                .iter()
                .map(|(v, d)| (v.to_string(), d.to_string()))
                .collect(),
        ),
        ("account" | "config" | "auth", ["use" | "switch"]) => prefixed(
            value_choices("profile", env)
                .into_iter()
                .map(|v| (v, String::new()))
                .collect(),
        ),
        (_, []) => prefixed(
            subcommands(canonical)
                .iter()
                .map(|(n, d)| (n.to_string(), d.to_string()))
                .collect(),
        ),
        _ => Vec::new(),
    }
}

/// Shell script that wires `<bin> __complete` into the given shell.
pub fn script(shell: &str, bin: &str) -> Option<String> {
    let body = match shell {
        "bash" => BASH,
        "zsh" => ZSH,
        "fish" => FISH,
        "powershell" | "pwsh" => POWERSHELL,
        _ => return None,
    };
    let func = bin.replace(['-', '.'], "_");
    Some(body.replace("{bin}", bin).replace("{func}", &func))
}

const BASH: &str = r#"# {bin} bash completion. Install:
#   {bin} completion bash > ~/.local/share/bash-completion/completions/{bin}
# or add to ~/.bashrc:  source <({bin} completion bash)
_{func}_complete() {
    local cur="${COMP_WORDS[COMP_CWORD]}"
    local IFS=$'\n'
    local out
    out=$("{bin}" __complete "${COMP_WORDS[@]:1:COMP_CWORD-1}" "$cur" 2>/dev/null) || return
    COMPREPLY=($(compgen -W "$(printf '%s\n' $out | cut -f1)" -- "$cur"))
}
complete -o default -F _{func}_complete {bin}
"#;

const ZSH: &str = r#"#compdef {bin}
# {bin} zsh completion. Install:
#   {bin} completion zsh > "${fpath[1]}/_{bin}"   then restart zsh
# or add to ~/.zshrc:  source <({bin} completion zsh)
_{func}() {
    local -a lines items
    lines=("${(@f)$("{bin}" __complete "${(@)words[2,CURRENT-1]}" "${words[CURRENT]}" 2>/dev/null)}")
    local line
    for line in $lines; do
        [[ -z $line ]] && continue
        if [[ $line == *$'\t'* ]]; then
            items+=("${${line%%$'\t'*}//:/\\:}:${line#*$'\t'}")
        else
            items+=("${line//:/\\:}")
        fi
    done
    (( ${#items} )) && _describe -t values '{bin}' items || _files
}
compdef _{func} {bin}
"#;

const FISH: &str = r#"# {bin} fish completion. Install:
#   {bin} completion fish > ~/.config/fish/completions/{bin}.fish
function __{func}_complete
    set -l words (commandline -opc)
    set -e words[1]
    {bin} __complete $words (commandline -ct) 2>/dev/null
end
complete -c {bin} -f -a '(__{func}_complete)'
"#;

const POWERSHELL: &str = r#"# {bin} PowerShell completion. Install (add to $PROFILE):
#   {bin} completion powershell | Out-String | Invoke-Expression
Register-ArgumentCompleter -Native -CommandName '{bin}' -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    $words = @($commandAst.CommandElements | Select-Object -Skip 1 | ForEach-Object { $_.ToString() })
    if ($wordToComplete -eq '') { $words += '' }
    & '{bin}' __complete @words 2>$null | ForEach-Object {
        $parts = $_ -split "`t", 2
        $desc = if ($parts.Count -gt 1) { $parts[1] } else { $parts[0] }
        [System.Management.Automation.CompletionResult]::new($parts[0], $parts[0], 'ParameterValue', $desc)
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &[&str]) -> Vec<String> {
        s.iter().map(|w| w.to_string()).collect()
    }

    fn values(out: &[String]) -> Vec<&str> {
        out.iter().map(|l| l.split('\t').next().unwrap()).collect()
    }

    #[test]
    fn top_level_lists_launch_and_api_commands() {
        let out = complete(&words(&[""]), &BTreeMap::new());
        let v = values(&out);
        for want in ["login", "claude", "grok", "api", "completion"] {
            assert!(v.contains(&want), "missing {want}: {v:?}");
        }
    }

    #[test]
    fn prefix_filters_candidates() {
        let out = complete(&words(&["gr"]), &BTreeMap::new());
        assert_eq!(values(&out), vec!["grok"]);
    }

    #[test]
    fn launch_flags_include_yolo() {
        let out = complete(&words(&["claude", "--y"]), &BTreeMap::new());
        assert!(values(&out).contains(&"--yolo"), "{out:?}");
    }

    #[test]
    fn effort_flag_offers_levels() {
        let out = complete(&words(&["claude", "--effort", ""]), &BTreeMap::new());
        assert!(values(&out).contains(&"high"), "{out:?}");
    }

    #[test]
    fn nothing_after_double_dash() {
        let out = complete(&words(&["claude", "--", ""]), &BTreeMap::new());
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn api_resource_then_verb() {
        let res = complete(&words(&["api", ""]), &BTreeMap::new());
        assert!(values(&res).contains(&"keys"), "{res:?}");
        let verbs = complete(&words(&["api", "keys", ""]), &BTreeMap::new());
        assert!(values(&verbs).contains(&"create"), "{verbs:?}");
    }

    #[test]
    fn every_shell_has_a_script_that_calls_the_engine() {
        for shell in SHELLS {
            let s = script(shell, "anyr").unwrap();
            assert!(
                s.contains("__complete") && s.contains("anyr"),
                "{shell}: {s}"
            );
        }
        assert!(script("tcsh", "anyr").is_none());
    }
}
