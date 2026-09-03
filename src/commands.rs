use std::collections::{BTreeMap, HashMap};

use crate::help::{command_help, resolve_bin, root_help, set_invoked_bin};
use crate::parse::{parse_cli_args, ParsedArgs};
use crate::term;
use crate::VERSION;

use crate::cmd::account::{run_account, run_logout};
use crate::cmd::auth::run_auth;
use crate::cmd::config_tui::run_config;
use crate::cmd::dispatch::{
    allowed_flags, assert_known_flags, canonical_command, help_topic, known_command,
    should_open_launcher, stub, tui_wants_dump, wants_help,
};
use crate::cmd::keys::run_keys;
use crate::cmd::launch::run_launch;
use crate::cmd::login::run_login;
use crate::cmd::menu::run_menu;
use crate::cmd::models::run_models;
use crate::cmd::usage::{run_usage, run_whoami};

pub fn run(argv: Vec<String>, env: HashMap<String, String>) -> i32 {
    let raw = if argv.first().map(String::as_str) == Some("--") {
        argv[1..].to_vec()
    } else {
        argv
    };
    let env: BTreeMap<String, String> = env.into_iter().collect();
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
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
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
        print!("{}", root_help());
        return 0;
    }
    if !known_command(command) {
        eprintln!(
            "Unknown command \"{command}\". Run \"{} --help\".",
            crate::help::invoked_bin()
        );
        return 1;
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
            eprintln!("{err}");
            return 1;
        }
    }
    match dispatch(canonical_command(command), &parsed, &env) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
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
        "models" => run_models(parsed, env),
        "usage" => run_usage(parsed, env),
        "whoami" | "status" => run_whoami(parsed, env),
        "config" => run_config(parsed, env),
        "account" => run_account(parsed, env),
        "keys" => run_keys(parsed, env),
        "menu" => run_menu(parsed, env),
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
        "cursor" | "cline" | "windsurf" => {
            print!("{}", command_help(command).unwrap_or_default());
            Ok(0)
        }
        "upgrade" | "update" => crate::upgrade::run(parsed, env),
        "onboard" | "impl" | "plan" | "fix" | "deploy" | "cp" => {
            crate::onboard::run(command, parsed)
        }
        _ => stub(command),
    }
}
