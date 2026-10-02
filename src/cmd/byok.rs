//! `anyr byok` — bring-your-own provider keys, optionally donated to the
//! community pool. Keys are read from `ANYR_BYOK_KEY`, piped stdin, or a
//! hidden prompt; they are never taken as arguments, echoed, or logged.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Read};

use serde_json::{json, Value};

use crate::api::{context, get, list, rows, send, Ctx};
use crate::cmd::dispatch::{usage_exit, USAGE};
use crate::parse::ParsedArgs;
use crate::term;

/// Env var holding the provider key for non-interactive `byok add`.
pub(crate) const KEY_ENV: &str = "ANYR_BYOK_KEY";

/// Mirrors `POOL_CONSENT_VERSION` / `POOL_CONSENT_TEXT` in the anyrouter repo
/// (`packages/lib/src/byok-pool-consent.ts`). The server rejects other versions.
const POOL_CONSENT_VERSION: &str = "v5";
const POOL_CONSENT_TEXT: &str = "By donating this key, you authorize AnyRouter to use it to serve \
requests from other users. Usage will appear on your provider account. You may withdraw at any \
time. Provider ToS may restrict this — you are responsible for compliance with your provider's \
terms. On paid coverage you set a USD-per-million price per model (from $0 up to 10% over the \
upstream list). Pool callers pay that price from AnyRouter credits. A model you leave unpriced is \
charged at your provider's list price, or, if you choose runtime dynamic pricing, at the cost \
your provider reports for each request, never more than the list price. For requests charged at \
a price you set or at runtime dynamic pricing, you are credited what the caller paid minus \
AnyRouter's platform fee of 10%. For other requests, your reward is based on your provider's \
list price for the requests your key serves.";

const DASHBOARD_BYOK: &str = "https://anyrouter.dev/dashboard/byok";

/// Where a stored provider key serves traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Byok,
    Donated,
    Unknown,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Byok => "byok",
            Kind::Donated => "donated",
            Kind::Unknown => "unknown",
        }
    }

    fn badge(self) -> &'static str {
        match self {
            Kind::Byok => "BYOK",
            Kind::Donated => "DONATED",
            Kind::Unknown => "?",
        }
    }
}

pub(crate) fn run_byok(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    usage_exit(run_inner(parsed, env))
}

fn run_inner(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<i32, String> {
    let args: Vec<&str> = parsed.passthrough.iter().map(String::as_str).collect();
    let donate = parsed.flag_true("donate");
    // `anyr byok --donate <provider>` means `add`.
    let (verb, rest) = match args.first().copied() {
        Some("list") | Some("ls") => ("list", &args[1..]),
        Some("add") | Some("new") => ("add", &args[1..]),
        Some(_) if donate => ("add", &args[..]),
        None if donate => ("add", &args[..]),
        None => ("list", &args[..]),
        Some(other) => {
            return Err(usage(&format!(
                "unknown byok command \"{other}\"; use: {} byok [list|add <provider> [--donate]]",
                crate::help::invoked_bin()
            )))
        }
    };
    match verb {
        "list" => run_list(&mgmt_ctx(parsed, env)?),
        _ => run_add(parsed, env, rest, donate),
    }
}

fn usage(msg: &str) -> String {
    format!("{USAGE}{} {msg}", term::err("error:"))
}

/// BYOK routes take a management key (`ak_…`), not an inference key.
fn mgmt_ctx(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> Result<Ctx, String> {
    let ctx = context(parsed, env);
    let Some(mk) = ctx.management_key.clone() else {
        return Err(format!(
            "{} byok needs a management key (ak_…); your API key cannot manage provider keys.\n{} create one at https://anyrouter.dev/settings/management-keys with read:byok and write:byok, then:\n      export ANYROUTER_MANAGEMENT_KEY=ak_…",
            term::err("error:"),
            term::dim("hint:")
        ));
    };
    Ok(Ctx {
        key: Some(mk),
        management_key: None,
        ..ctx
    })
}

/// `kind` from the server when present; otherwise an active row in the
/// donated list; `Unknown` when that list could not be read.
pub(crate) fn classify(key: &Value, donated: Option<&[Value]>) -> Kind {
    match key.get("kind").and_then(Value::as_str) {
        Some("donated") => return Kind::Donated,
        Some("byok") => return Kind::Byok,
        _ => {}
    }
    let Some(donated) = donated else {
        return Kind::Unknown;
    };
    let id = key.get("id").and_then(Value::as_str);
    let active = donated.iter().any(|d| {
        d.get("byok_key_id").and_then(Value::as_str) == id
            && id.is_some()
            && d.get("status").and_then(Value::as_str) != Some("revoked")
    });
    if active {
        Kind::Donated
    } else {
        Kind::Byok
    }
}

fn run_list(ctx: &Ctx) -> Result<i32, String> {
    let keys = rows(&get(ctx, "/v1/auth/byok?source=cli", true)?);
    let needs_join = keys.iter().any(|k| k.get("kind").is_none());
    let donated = if needs_join {
        get(ctx, "/v1/byok/donated?source=cli", true)
            .ok()
            .map(|v| rows(&v))
    } else {
        Some(Vec::new())
    };
    let mut unknown = false;
    let out: Vec<Value> = keys
        .into_iter()
        .map(|mut k| {
            let kind = classify(&k, donated.as_deref());
            unknown |= kind == Kind::Unknown;
            let label = if ctx.json {
                kind.as_str()
            } else {
                kind.badge()
            };
            if let Some(obj) = k.as_object_mut() {
                obj.insert("kind".into(), Value::String(label.into()));
            }
            k
        })
        .collect();
    let code = list(
        ctx,
        Value::Array(out),
        &["kind", "provider_id", "id", "label", "key_preview"],
    )?;
    if unknown {
        eprintln!(
            "{}",
            term::dim("note: could not read donated keys; source shown as ? (see the dashboard)")
        );
    }
    Ok(code)
}

/// Read the provider key: env var, else piped stdin, else hidden prompt.
fn read_key(env: &BTreeMap<String, String>) -> Result<String, String> {
    if let Some(v) = env.get(KEY_ENV).filter(|v| !v.trim().is_empty()) {
        return Ok(v.trim().to_string());
    }
    if !std::io::stdin().is_terminal() {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("could not read the provider key from stdin: {e}"))?;
        return Ok(buf.trim().to_string());
    }
    term::prompt_secret("Provider API key (hidden): ").map(|s| s.trim().to_string())
}

/// Strip the secret from any text that may reach the terminal.
fn redact(text: String, secret: &str) -> String {
    if secret.len() >= 4 {
        text.replace(secret, "[redacted]")
    } else {
        text
    }
}

fn run_add(
    parsed: &ParsedArgs,
    env: &BTreeMap<String, String>,
    rest: &[&str],
    donate: bool,
) -> Result<i32, String> {
    let bin = crate::help::invoked_bin();
    let Some(provider) = rest.first().copied() else {
        return Err(usage(&format!(
            "missing <provider>; usage: {bin} byok add <provider> [--donate] [--yes]"
        )));
    };
    if rest.len() > 1 {
        // Never echo the extra argument: it is likely the key itself.
        return Err(usage(&format!(
            "the provider key is not accepted as an argument (it would land in shell history). \
Pipe it on stdin, set {KEY_ENV}, or enter it at the prompt."
        )));
    }
    let yes = parsed.flag_true("yes");
    // Usage checks before any read or request: a donation needs consent.
    if donate && !yes && !term::is_interactive() {
        return Err(usage(
            "donating needs confirmation; re-run on a terminal or pass --yes to accept the pool terms",
        ));
    }
    let ctx = mgmt_ctx(parsed, env)?;
    let secret = read_key(env)?;
    if secret.is_empty() {
        return Err(usage(&format!(
            "no provider key given; pipe it on stdin, set {KEY_ENV}, or enter it at the prompt"
        )));
    }
    if donate {
        eprintln!("{}\n", POOL_CONSENT_TEXT);
        if !yes
            && !term::confirm(&format!(
                "Donate this {provider} key to the community pool?"
            ))
        {
            eprintln!("{}", term::dim("Cancelled. Nothing was sent."));
            return Ok(1);
        }
    }
    let kind = if donate { Kind::Donated } else { Kind::Byok };
    let body = json!({
        "provider_id": provider,
        "api_key": secret,
        "source": "cli",
        "kind": kind.as_str(),
    });
    let created = send(&ctx, "POST", "/v1/auth/byok", Some(&body.to_string()), true)
        .map_err(|e| redact(e, &secret))?;
    let id = created
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let preview = created
        .get("key_preview")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !donate {
        println!(
            "{}  {provider} {preview}  {}  {}",
            term::ok("Added"),
            Kind::Byok.badge(),
            term::dim(&id)
        );
        return Ok(0);
    }
    if id.is_empty() {
        return Err(format!(
            "{} key saved as private BYOK, but the server returned no id to donate. Donate it from {DASHBOARD_BYOK}",
            term::err("error:")
        ));
    }
    let donation = json!({
        "byok_key_id": id,
        "consent": POOL_CONSENT_VERSION,
        "source": "cli",
        "kind": "donated",
    });
    match send(
        &ctx,
        "POST",
        "/v1/byok/donated",
        Some(&donation.to_string()),
        true,
    ) {
        Ok(_) => {
            println!(
                "{}  {provider} {preview}  {}  {}",
                term::ok("Donated"),
                Kind::Donated.badge(),
                term::dim(&id)
            );
            Ok(0)
        }
        // Keep only the status line: the generic `login` hint is wrong
        // here, since no CLI credential can donate yet.
        Err(e) => Err(format!(
            "{}\n{} the key was saved as a private BYOK key ({id}) but not donated. Donate it from {DASHBOARD_BYOK}",
            redact(e, &secret).lines().next().unwrap_or_default(),
            term::dim("note:")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_prefers_server_kind_then_active_donation() {
        let key = json!({"id": "byok_1", "kind": "donated"});
        assert_eq!(classify(&key, None), Kind::Donated);
        let key = json!({"id": "byok_1"});
        let donated = vec![json!({"byok_key_id": "byok_1", "status": "active"})];
        assert_eq!(classify(&key, Some(&donated)), Kind::Donated);
        let revoked = vec![json!({"byok_key_id": "byok_1", "status": "revoked"})];
        assert_eq!(classify(&key, Some(&revoked)), Kind::Byok);
    }

    #[test]
    fn classify_unknown_when_donations_unreadable() {
        // WHY: labelling a key BYOK when we could not check would tell a
        // donor their key is private when it may be serving the pool.
        assert_eq!(classify(&json!({"id": "byok_1"}), None), Kind::Unknown);
    }

    #[test]
    fn redact_removes_secret_from_server_errors() {
        let msg = "HTTP 400: invalid key sk-proj-abcdef123".to_string();
        assert!(!redact(msg, "sk-proj-abcdef123").contains("sk-proj"));
    }
}
