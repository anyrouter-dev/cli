//! `anyr whoami` / `anyr status` / `anyr auth status`: who is signed in, which
//! key and default model are active, plus a small action picker on a TTY.

use std::collections::BTreeMap;
use std::path::Path;

use crate::cmd::account::run_logout;
use crate::cmd::auth::run_auth_switch;
use crate::cmd::dispatch::{config_path, hint};
use crate::cmd::models::{models_for_picker, pick_model, save_model_slot};
use crate::http::{fetch_me, MeInfo};
use crate::key::{
    config_for_credential, load_config_if_present, mask_api_key, profile_or_default,
    resolve_api_key, resolve_base_url,
};
use crate::parse::{get_string_flag, ParsedArgs};
use crate::spawn::{catalog_model_id, display_model_id, session_model_label};
use crate::term;

const DASHBOARD_URL: &str = "https://anyrouter.dev/dashboard";

/// What the status view shows. Model tier mappings and the default tool stay
/// in `config`; they are not part of this summary.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StatusView {
    pub account: String,
    pub name: Option<String>,
    pub email: Option<String>,
    /// Why name and email are missing, when they are.
    pub identity_note: Option<String>,
    pub key_masked: String,
    pub default_model: String,
    pub config_path: String,
    pub base_url: String,
}

impl StatusView {
    pub(crate) fn identity_line(&self) -> String {
        match (self.name.as_deref(), self.email.as_deref()) {
            (Some(n), Some(e)) => format!("{n} <{e}>"),
            (Some(n), None) => n.to_string(),
            (None, Some(e)) => e.to_string(),
            (None, None) => self
                .identity_note
                .clone()
                .unwrap_or_else(|| "name and email unavailable".into()),
        }
    }

    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "account": self.account,
            "user": { "name": self.name, "email": self.email },
            "key_masked": self.key_masked,
            "default_model": self.default_model,
            "config_path": self.config_path,
            "base_url": self.base_url,
        })
    }

    /// Text starts with a plain visible character, never an escape or `\r`.
    pub(crate) fn render(&self) -> String {
        let who = if self.name.is_some() || self.email.is_some() {
            format!("Signed in as {}", term::accent(&self.identity_line()))
        } else {
            format!("Signed in ({})", self.identity_line())
        };
        let row = |label: &str, value: String| format!("{}  {value}\n", term::dim(label));
        let mut out = format!("{who}\n\n");
        out.push_str(&row("account      ", term::accent(&self.account)));
        out.push_str(&row("api key      ", self.key_masked.clone()));
        out.push_str(&row(
            "default model",
            term::model_id(&session_model_label(&self.default_model)),
        ));
        out.push_str(&row("config       ", self.config_path.clone()));
        out
    }
}

type Identity = (Option<String>, Option<String>, Option<String>);

/// (name, email, note on why identity is missing) from the account lookup.
pub(crate) fn identity_from_me(me: Result<MeInfo, String>) -> Identity {
    match me {
        Ok(me) => {
            let name = me
                .name
                .filter(|s| !s.is_empty())
                .or(me.username.filter(|s| !s.is_empty()));
            let email = me.email.filter(|s| !s.is_empty());
            let note = (name.is_none() && email.is_none())
                .then(|| "this key has no name or email on file".to_string());
            (name, email, note)
        }
        Err(err) => (
            None,
            None,
            Some(format!("name and email unavailable: {err}")),
        ),
    }
}

fn build_view(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &Path,
) -> Result<StatusView, String> {
    let cfg = config_for_credential(path, &parsed.flags, env)?;
    let account = get_string_flag(&parsed.flags, "profile")
        .or_else(|| env.get("ANYROUTER_PROFILE").cloned())
        .unwrap_or_else(|| cfg.active_profile.clone());
    let profile = profile_or_default(&cfg, &account, &parsed.flags, env)?;
    let key = resolve_api_key(&parsed.flags, env, Some(&profile)).ok_or_else(|| {
        crate::exit::tag(crate::exit::AUTH, hint("Not signed in. Run: {bin} login"))
    })?;
    let base_url = resolve_base_url(&parsed.flags, Some(&profile));
    let (name, email, identity_note) = identity_from_me(fetch_me(&base_url, &key));
    Ok(StatusView {
        account,
        name,
        email,
        identity_note,
        key_masked: mask_api_key(Some(&key)),
        default_model: display_model_id(profile.default_model()),
        config_path: path.display().to_string(),
        base_url,
    })
}

pub(crate) fn run_whoami(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
) -> Result<i32, String> {
    let path = config_path(parsed, env);
    let view = build_view(parsed, env, &path)?;
    if parsed.flag_true("json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&view.json()).unwrap_or_else(|_| "{}".into())
        );
        return Ok(0);
    }
    print!("{}", view.render());
    if term::machine_output(parsed, env) || !term::is_interactive() {
        return Ok(0);
    }
    status_picker(parsed, env, &path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusAction {
    SwitchAccount,
    ChangeModel,
    Logout,
    Dashboard,
    Quit,
}

pub(crate) const STATUS_ACTIONS: [(StatusAction, &str); 5] = [
    (StatusAction::SwitchAccount, "Switch account"),
    (StatusAction::ChangeModel, "Change default model"),
    (StatusAction::Logout, "Log out"),
    (StatusAction::Dashboard, "Open dashboard"),
    (StatusAction::Quit, "Quit"),
];

pub(crate) fn status_action_at(index: usize) -> StatusAction {
    STATUS_ACTIONS
        .get(index)
        .map(|(a, _)| *a)
        .unwrap_or(StatusAction::Quit)
}

fn status_picker(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &Path,
) -> Result<i32, String> {
    let labels: Vec<String> = STATUS_ACTIONS.iter().map(|(_, l)| l.to_string()).collect();
    loop {
        let Ok(idx) = term::pick("Account", &labels, None) else {
            return Ok(0);
        };
        let action = status_action_at(idx);
        if action == StatusAction::Quit {
            return Ok(0);
        }
        match run_status_action(action, parsed, env, path) {
            Ok(()) => {}
            Err(err) if err == "Cancelled." => {}
            Err(err) => eprintln!("{}", term::err(&err)),
        }
        if action == StatusAction::Logout {
            return Ok(0);
        }
        if let Ok(view) = build_view(parsed, env, path) {
            print!("\n{}", view.render());
        }
    }
}

fn run_status_action(
    action: StatusAction,
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    path: &Path,
) -> Result<(), String> {
    let bare = ParsedArgs {
        command: parsed.command.clone(),
        flags: parsed.flags.clone(),
        passthrough: Vec::new(),
    };
    match action {
        StatusAction::SwitchAccount => run_auth_switch(&bare, env).map(|_| ()),
        StatusAction::Logout => {
            if !term::confirm("Log out of this account?") {
                return Ok(());
            }
            run_logout(&bare, env).map(|_| ())
        }
        StatusAction::Dashboard => {
            if !crate::auth::open_url(DASHBOARD_URL) {
                println!("Open {}", term::link(DASHBOARD_URL));
            }
            Ok(())
        }
        StatusAction::ChangeModel => {
            let existing = load_config_if_present(path);
            let profile = existing
                .as_ref()
                .and_then(|c| c.profiles.get(&c.active_profile));
            let key = resolve_api_key(&parsed.flags, env, profile);
            let base = resolve_base_url(&parsed.flags, profile);
            let models = models_for_picker(&base, key.as_deref(), env);
            let current = profile.map(|p| p.default_model().to_string());
            let id = pick_model(&models, current.as_deref(), "Default model")?;
            save_model_slot(existing, path, "default", &catalog_model_id(&id)).map(|_| ())
        }
        StatusAction::Quit => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(name: Option<&str>, email: Option<&str>) -> StatusView {
        StatusView {
            account: "default".into(),
            name: name.map(Into::into),
            email: email.map(Into::into),
            identity_note: None,
            key_masked: "sk-ar-v1-qjd4c…e5bu".into(),
            default_model: "poolside/laguna-s-2.1".into(),
            config_path: "/home/u/.anyrouter/config.yaml".into(),
            base_url: "https://anyrouter.dev/api".into(),
        }
    }

    #[test]
    fn text_shows_name_email_and_no_tier_rows() {
        let out = view(Some("Duyet Le"), Some("me@duyet.net")).render();
        assert!(out.contains("Duyet Le"), "{out}");
        assert!(out.contains("me@duyet.net"), "{out}");
        assert!(out.contains("poolside/laguna-s-2.1"), "{out}");
        for gone in [
            "claude_",
            "default_tool",
            "haiku",
            "sonnet",
            "opus",
            "fable",
        ] {
            assert!(!out.contains(gone), "{gone} still shown:\n{out}");
        }
    }

    #[test]
    fn first_byte_is_visible_text() {
        let out = view(Some("A"), Some("a@b.c")).render();
        assert!(
            !out.starts_with('\r') && !out.starts_with('\x1b'),
            "{out:?}"
        );
        let anon = view(None, None).render();
        assert!(anon.starts_with("Signed in"), "{anon:?}");
    }

    #[test]
    fn missing_identity_says_why() {
        let (n, e, note) = identity_from_me(Err("HTTP 401".into()));
        assert!(n.is_none() && e.is_none());
        let mut v = view(None, None);
        v.identity_note = note;
        assert!(v.render().contains("HTTP 401"), "{}", v.render());
        let (_, _, note) = identity_from_me(Ok(MeInfo::default()));
        assert!(note.unwrap().contains("no name or email"));
    }

    #[test]
    fn username_stands_in_for_a_missing_name() {
        let me = MeInfo {
            username: Some("duyet".into()),
            email: Some("me@duyet.net".into()),
            ..Default::default()
        };
        let (n, e, _) = identity_from_me(Ok(me));
        assert_eq!(n.as_deref(), Some("duyet"));
        assert_eq!(e.as_deref(), Some("me@duyet.net"));
    }

    #[test]
    fn json_has_the_documented_shape() {
        let v = view(Some("Duyet"), Some("me@duyet.net")).json();
        assert_eq!(v["account"], "default");
        assert_eq!(v["user"]["name"], "Duyet");
        assert_eq!(v["user"]["email"], "me@duyet.net");
        assert_eq!(v["key_masked"], "sk-ar-v1-qjd4c…e5bu");
        assert_eq!(v["default_model"], "poolside/laguna-s-2.1");
        assert!(v["config_path"].is_string());
        assert!(v.get("claude_haiku").is_none() && v.get("default_tool").is_none());
    }

    #[test]
    fn picker_indexes_map_to_the_documented_actions() {
        let labels: Vec<_> = STATUS_ACTIONS.iter().map(|(_, l)| *l).collect();
        assert_eq!(
            labels,
            [
                "Switch account",
                "Change default model",
                "Log out",
                "Open dashboard",
                "Quit"
            ]
        );
        assert_eq!(status_action_at(0), StatusAction::SwitchAccount);
        assert_eq!(status_action_at(1), StatusAction::ChangeModel);
        assert_eq!(status_action_at(2), StatusAction::Logout);
        assert_eq!(status_action_at(3), StatusAction::Dashboard);
        assert_eq!(status_action_at(4), StatusAction::Quit);
        assert_eq!(status_action_at(99), StatusAction::Quit);
    }
}
