use std::collections::{BTreeMap, HashMap};

use crate::help::{command_help, commands_help, resolve_bin, root_help, set_invoked_bin};
use crate::parse::{parse_cli_args, ParsedArgs};
use crate::term;
use crate::VERSION;

use crate::cmd::account::{run_account, run_logout};
use crate::cmd::auth::run_auth;
use crate::cmd::byok::run_byok;
use crate::cmd::config_tui::run_config;
use crate::cmd::decision::run_decision;
use crate::cmd::dispatch::{
    allowed_flags, assert_known_flags, canonical_command, help_topic, known_command,
    should_open_launcher, stub, suggest_command, tui_wants_dump, usage_exit, wants_help, USAGE,
};
use crate::cmd::keys::run_keys;
use crate::cmd::launch::run_launch;
use crate::cmd::login::run_login;
use crate::cmd::menu::run_menu;
use crate::cmd::models::run_models_cli;
use crate::cmd::usage::{run_usage, run_whoami};

pub fn run(argv: Vec<String>, env: HashMap<String, String>) -> i32 {
    let raw = if argv.first().map(String::as_str) == Some("--") {
        argv[1..].to_vec()
    } else {
        argv
    };
    let env: BTreeMap<String, String> = env.into_iter().collect();
    // Completion callback runs before parsing: partial words like `--model`
    // with no value must not error, and nothing may hit the network.
    if raw.first().map(String::as_str) == Some("__complete") {
        for line in crate::completion::complete(&raw[1..], &env) {
            println!("{line}");
        }
        return 0;
    }
    #[cfg(not(target_arch = "wasm32"))]
    let argv0 = std::env::args().next();
    #[cfg(target_arch = "wasm32")]
    let argv0: Option<String> = None;
    set_invoked_bin(resolve_bin(
        argv0.as_deref(),
        env.get("ANYR_DISPLAY_BIN").map(String::as_str),
    ));

    let parsed = match parse_cli_args(&raw) {
        Ok(p) => p,
        Err(err) => return usage_fail(&err),
    };

    let command = parsed.command.as_str();
    if command == "--version" || command == "-v" {
        println!("{VERSION} (built {})", crate::buildinfo::display_time());
        return 0;
    }

    // parse_cli_args maps empty argv to command "help". Check emptiness first
    // so a real terminal gets the TUI launcher, not --help. Dump mode also
    // opens the launcher without a TTY (`ANYR_TUI_DUMP=1`).
    if should_open_launcher(&raw, term::is_interactive(), tui_wants_dump(&parsed, &env)) {
        #[cfg(feature = "native")]
        crate::upgrade::on_startup("menu", &parsed, &env);
        let empty = ParsedArgs {
            command: "menu".into(),
            flags: parsed.flags.clone(),
            passthrough: Vec::new(),
        };
        return match run_menu(&empty, &env) {
            Ok(code) => code,
            Err(err) => {
                eprintln!("{err}");
                1
            }
        };
    }

    #[cfg(feature = "native")]
    crate::upgrade::on_startup(command, &parsed, &env);

    if raw.is_empty() || command == "help" || command == "--help" || command == "-h" {
        if let Some(bad) = unknown_help_topic(&parsed.passthrough) {
            return help_topic_fail(&bad);
        }
        let topic = parsed.passthrough.first().map(String::as_str);
        if topic == Some("commands") || topic == Some("help") {
            print!("{}", commands_help());
        } else if let Some(help) = topic_help(&parsed.passthrough) {
            print!("{help}");
        } else {
            print!("{}", root_help());
        }
        return 0;
    }
    if !known_command(command) {
        let bin = crate::help::invoked_bin();
        eprintln!("{} unknown command \"{command}\"", term::err("error:"));
        match suggest_command(command) {
            Some(near) => eprintln!("{} did you mean `{bin} {near}`?", term::dim("hint:")),
            None => eprintln!("{} run `{bin} --help`", term::dim("hint:")),
        }
        return 2;
    }
    if wants_help(&parsed) {
        let topic = help_topic(&parsed);
        if let Some(help) = command_help(&topic).or_else(|| {
            if parsed.command == "auth" {
                command_help("auth")
            } else {
                None
            }
        }) {
            print!("{help}");
            return 0;
        }
    }
    if let Some(allowed) = allowed_flags(command) {
        if let Err(err) = assert_known_flags(command, &parsed.flags, allowed) {
            return usage_fail(&err);
        }
    }
    let canonical = canonical_command(command);
    let mut parsed = parsed.clone();
    // Login handles `--key` itself (acquire_api_key); `auth login` is the same.
    let own_key_handling = matches!(canonical, "login" | "setup")
        || (canonical == "auth"
            && matches!(
                parsed.passthrough.first().map(String::as_str),
                Some("login" | "setup")
            ));
    if !own_key_handling {
        if let Err(err) = crate::key::normalize_key_flag(
            &mut parsed.flags,
            &mut std::io::stdin().lock(),
            &mut std::io::stderr(),
        ) {
            return usage_fail(&err);
        }
    }
    match usage_exit(dispatch(canonical, &parsed, &env)) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

/// The one exit path for usage errors (bad flag, bad subcommand, bad topic).
fn usage_fail(message: &str) -> i32 {
    eprintln!("{message}");
    2
}

/// First word of `help <words>` that is not a real topic, if any.
fn unknown_help_topic(words: &[String]) -> Option<String> {
    let first = words.first()?;
    let known = |w: &str| known_command(w) || command_help(w).is_some();
    if !known(first) {
        return Some(first.clone());
    }
    if first == "auth" {
        return words.get(1).filter(|w| !known(w)).cloned();
    }
    None
}

/// Help for `help <topic>` / `help auth <sub>`; None falls back to root help.
fn topic_help(words: &[String]) -> Option<String> {
    let first = words.first()?;
    if first == "auth" {
        if let Some(help) = words.get(1).and_then(|sub| command_help(sub)) {
            return Some(help);
        }
    }
    command_help(first)
}

fn help_topic_fail(topic: &str) -> i32 {
    let bin = crate::help::invoked_bin();
    eprintln!("{} unknown help topic \"{topic}\"", term::err("error:"));
    match suggest_command(topic) {
        Some(near) => eprintln!("{} did you mean `{bin} help {near}`?", term::dim("hint:")),
        None => eprintln!("{} run `{bin} --help`", term::dim("hint:")),
    }
    2
}

fn dispatch(
    command: &str,
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    match command {
        "auth" => run_auth(parsed, env),
        "login" | "setup" => run_login(parsed, env),
        "logout" => run_logout(parsed, env),
        "models" => run_models_cli(parsed, env),
        "usage" => run_usage(parsed, env),
        "whoami" | "status" => run_whoami(parsed, env),
        "config" => run_config(parsed, env),
        "decision" => run_decision(parsed, env),
        "account" => run_account(parsed, env),
        "keys" => run_keys(parsed, env),
        "byok" => run_byok(parsed, env),
        "menu" => run_menu(parsed, env),
        "commands" => {
            print!("{}", commands_help());
            Ok(0)
        }
        "claude" | "codex" | "grok" | "opencode" | "pool" | "pi" => {
            run_launch(command, parsed, env)
        }
        "relay" => {
            #[cfg(feature = "native")]
            {
                crate::relay::run(parsed, env)
            }
            #[cfg(not(feature = "native"))]
            {
                let _ = parsed;
                stub("relay")
            }
        }
        "cursor" | "cline" | "windsurf" => stub(command),
        "upgrade" | "update" => crate::upgrade::run(parsed, env),
        "api" => crate::api::run(parsed, env),
        "completion" => run_completion(parsed),
        "onboard" | "impl" | "plan" | "fix" | "deploy" | "cp" => {
            crate::onboard::run(command, parsed)
        }
        _ => stub(command),
    }
}

fn run_completion(parsed: &ParsedArgs) -> Result<i32, String> {
    let bin = crate::help::invoked_bin();
    let shell = parsed.passthrough.first().map(String::as_str).unwrap_or("");
    match crate::completion::script(shell, &bin) {
        Some(script) => {
            print!("{script}");
            Ok(0)
        }
        None => Err(format!(
            "{USAGE}Usage: {bin} completion <{}>",
            crate::completion::SHELLS.join("|")
        )),
    }
}
